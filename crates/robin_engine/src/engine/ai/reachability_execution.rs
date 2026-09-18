//! Route-failure decisions against current owner and sequence state.

use super::*;
use crate::ai::{AiLockFlags, AiState, BodyReaction, DutyFlags, Stimulus, Substate};
use crate::ai_enemy::{SeekFlags, UNDEFINED_DIRECTION};

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_ai_reachability_failure(&mut self) {
        match self
            .engine
            .ai(self.owner, "route failure substate")
            .current_substate
        {
            Substate::SeekingSeekpoint => self.execute_ai_seek_next_point(),
            Substate::SeekingBody => {
                self.execute_ai_body_reaction(BodyReaction::Unreachable);
            }
            Substate::FleeingPanic => {
                self.execute_ai_panic_segment(StimulusType::EventCouldntReachPoint);
            }
            Substate::AttackingObserve => {}
            _ => self.execute_ai_reachability_emergency(),
        }
    }

    fn execute_ai_reachability_emergency(&mut self) {
        if self.engine.is_very_very_busy(self.owner) {
            let ai = self.engine.ai_mut(self.owner, "busy route failure");
            ai.non_script_lock(AiLockFlags::BUSY);
            ai.was_busy = true;
            self.execute_ai_callback(&Stimulus::new(StimulusType::EventCouldntReachPoint));
            return;
        }
        match self
            .engine
            .ai(self.owner, "route failure state")
            .current_state
        {
            AiState::Sleeping
            | AiState::Default
            | AiState::Wondering
            | AiState::Menacing
            | AiState::Fleeing => {
                self.execute_ai_return_to_duty(DutyFlags::BECAUSE_COULDNT_REACHPOINT);
            }
            AiState::Seeking => {
                self.execute_ai_seek_area(
                    self.engine.live_ai_position(self.owner),
                    crate::parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                    SeekFlags::empty(),
                    UNDEFINED_DIRECTION,
                );
            }
            AiState::Attacking => {
                let swordfighting = !self
                    .engine
                    .expect_entity(self.owner, "route failure combat")
                    .human_data()
                    .expect("route failure owner must be human")
                    .opponents
                    .is_empty();
                if swordfighting {
                    self.duty_set_state(AiState::Attacking, Substate::AttackingSwordfight);
                    self.engine
                        .world
                        .entities
                        .expect_ai_controller_mut(
                            self.owner,
                            format_args!("route failure combat timer"),
                        )
                        .launch_timer(20, self.engine.control.frame_counter);
                } else {
                    self.execute_ai_get_battle_overview(0);
                }
            }
        }
    }
}
