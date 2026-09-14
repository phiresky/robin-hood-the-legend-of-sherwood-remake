use super::*;
use crate::ai::{
    AiAdmission, AiState, AlertFlags, AlertLevel, EmoticonType, LogLineType, Stimulus,
    StimulusType, Substate,
};
use crate::element::{EyeStatus, Posture};
use crate::sim_rng::SimulationContext;

#[cfg(test)]
mod tests;

impl EngineInner {
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
        admission: &AiAdmission,
    ) -> bool {
        let frozen = self.ai.global.freeze;
        let ai = self.observation_ai_mut(owner);
        ai.base.cached_frame = admission.frame;
        ai.base.cached_in_building = admission.in_building;
        ai.base.debug_macro_lifecycle_at(
            admission.frame,
            admission.original_creation_order,
            "think_enter",
            stimulus.stimulus_type,
        );
        if !ai.base.admit_think_before_role_gates(stimulus, frozen) {
            if stimulus.stimulus_type == StimulusType::EventAfterScriptGoOn {
                ai.base.outbox.reentrant.engine_drains_after_script_go_on = false;
            }
            return false;
        }
        if admission.self_is_unconscious {
            let refused = match stimulus.stimulus_type {
                StimulusType::EventLoseConsciousness => None,
                StimulusType::EventFitAgain if admission.posture != Posture::Carried => None,
                StimulusType::EventFitAgain => Some(7),
                _ => Some(8),
            };
            if let Some(reason) = refused {
                ai.base.register_log_line(LogLineType::EventRefused, reason);
                if stimulus.stimulus_type == StimulusType::EventAfterScriptGoOn {
                    ai.base.outbox.reentrant.engine_drains_after_script_go_on = false;
                }
                return false;
            }
        }
        if !ai.base.admit_think_after_role_gates(stimulus, admission) {
            if stimulus.stimulus_type == StimulusType::EventAfterScriptGoOn {
                ai.base.outbox.reentrant.engine_drains_after_script_go_on = false;
            }
            return false;
        }
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
        ai.base.break_macro();
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        if stimulus.stimulus_type == StimulusType::EventLoseConsciousness {
            let ai = self.observation_ai_mut(owner);
            ai.base.clear_emoticon();
            if ai.base.current_substate.is_take_money()
                || ai.base.current_substate.is_fight_for_money()
            {
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
                self.forget_ai_nearby_coins_live(owner);
            }
        } else if stimulus.stimulus_type == StimulusType::EventWasp {
            self.observation_ai_mut(owner)
                .base
                .set_emoticon(EmoticonType::Thunderstorm);
        }
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        self.duty_set_state(sim, assets, owner, state, substate);
        let actor = self
            .world
            .entities
            .expect_ai_actor_data_mut(owner, format_args!("enemy admission eyes"));
        crate::ai_vision::set_view_status(actor, eyes);
        let ai = self.observation_ai_mut(owner);
        if stimulus.stimulus_type == StimulusType::EventLoseConsciousness {
            ai.set_alert_status_with_flags(AlertLevel::Green, AlertFlags::INSTANT_MUSIC_CHANGE);
        }
        ai.base.sorrow_level = 0;
        ai.attentive = false;
        ai.will_be_attentive = false;
        ai.base
            .register_log_line(LogLineType::EventRefused, refused);
        false
    }
}
