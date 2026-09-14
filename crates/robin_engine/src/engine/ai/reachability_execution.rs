//! Route-failure decisions against current owner and sequence state.

use super::*;
use crate::ai::{AiLockFlags, AiState, BodyReaction, DutyFlags, Stimulus, Substate};
use crate::ai_enemy::{SeekFlags, UNDEFINED_DIRECTION};
use crate::sim_rng::SimulationContext;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_reachability_failure(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        match self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("route failure substate"))
            .current_substate
        {
            Substate::SeekingSeekpoint => self.execute_ai_seek_next_point(sim, assets, owner),
            Substate::SeekingBody => {
                self.execute_ai_body_reaction(sim, assets, owner, BodyReaction::Unreachable);
            }
            Substate::FleeingPanic => {
                self.execute_ai_panic_segment(
                    sim,
                    assets,
                    owner,
                    StimulusType::EventCouldntReachPoint,
                );
            }
            Substate::AttackingObserve => {}
            _ => self.execute_ai_reachability_emergency(sim, assets, owner),
        }
    }

    fn execute_ai_reachability_emergency(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        if self.is_very_very_busy(owner) {
            let ai = self
                .world
                .entities
                .expect_ai_controller_mut(owner, format_args!("busy route failure"));
            ai.non_script_lock(AiLockFlags::BUSY);
            ai.was_busy = true;
            self.execute_ai_callback(
                sim,
                assets,
                owner,
                &Stimulus::new(StimulusType::EventCouldntReachPoint),
            );
            return;
        }
        match self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("route failure state"))
            .current_state
        {
            AiState::Sleeping
            | AiState::Default
            | AiState::Wondering
            | AiState::Menacing
            | AiState::Fleeing => {
                self.execute_ai_return_to_duty(
                    sim,
                    assets,
                    owner,
                    DutyFlags::BECAUSE_COULDNT_REACHPOINT,
                );
            }
            AiState::Seeking => {
                self.execute_ai_seek_area(
                    sim,
                    assets,
                    owner,
                    self.live_ai_position(owner),
                    crate::parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                    SeekFlags::empty(),
                    UNDEFINED_DIRECTION,
                );
            }
            AiState::Attacking => {
                let swordfighting = !self
                    .expect_entity(owner, "route failure combat")
                    .human_data()
                    .expect("route failure owner must be human")
                    .opponents
                    .is_empty();
                if swordfighting {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Attacking,
                        Substate::AttackingSwordfight,
                    );
                    self.world
                        .entities
                        .expect_ai_controller_mut(owner, format_args!("route failure combat timer"))
                        .launch_timer(20, self.control.frame_counter);
                } else {
                    self.execute_ai_get_battle_overview(sim, assets, owner, 0);
                }
            }
        }
    }
}
