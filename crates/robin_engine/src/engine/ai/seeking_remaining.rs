//! Search observation callbacks with live state across calls.

use super::*;
use crate::ai::{AiState, DutyFlags, GotoFlags, LookDirection, Stimulus, StimulusType, Substate};
use crate::ai_enemy::{ProfileRank, SeekFlags, UNDEFINED_DIRECTION};

impl EngineInner {
    fn remaining_search_timer(&mut self, owner: EntityId, frames: u32) {
        let frame = self.control.frame_counter;
        self.ai_mut(owner, "search timer")
            .launch_timer(frames, frame);
    }
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_ai_remaining_search_event(
        &mut self,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        use StimulusType::*;
        use Substate::*;
        let substate = self
            .engine
            .ai(self.owner, "search callback substate")
            .current_substate;
        match (substate, stimulus.stimulus_type) {
            (FleeingRunForArrowReserves, EventReachPoint) => {
                self.engine
                    .ai_actor_mut(self.owner, "arrow reserve refill")
                    .number_of_arrows = crate::parameters_ai::MAX_NPC_ARROWS as u16;
                let position = self
                    .engine
                    .ai(self.owner, "arrow reserve search")
                    .seek_position;
                self.execute_ai_seek_area(
                    position,
                    crate::parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                    SeekFlags::LOCATION_FIRST,
                    UNDEFINED_DIRECTION,
                );
            }
            (SeekingArrowReactiontime, EventTimer) => {
                self.execute_ai_speech(crate::ai::AiSpeechAttempt {
                    remark: crate::ai::Remark::Arrow,
                    flags: 0,
                });
                self.duty_set_state(AiState::Seeking, SeekingArrow);
                let position = self
                    .engine
                    .ai(self.owner, "arrow reaction destination")
                    .seek_position;
                self.duty_go_to(position, GotoFlags::RUN);
                let position = self
                    .engine
                    .ai(self.owner, "arrow reaction broadcast")
                    .seek_position;
                self.engine
                    .execute_ai_look_there(self.sim, self.assets, self.owner, position, 100);
                self.engine.remaining_search_timer(self.owner, 200);
            }
            (SeekingArrow, EventTimer | EventReachPoint) => {
                let mut flags = SeekFlags::LOCATION_FIRST | SeekFlags::WALKING;
                if self
                    .engine
                    .enemy_ai(self.owner, "arrow search rank")
                    .get_rank(&self.assets.profile_manager)
                    == ProfileRank::Soldier
                {
                    flags |= SeekFlags::LOOK_FOR_HELP_AFTER;
                }
                let position = self.engine.live_ai_position(self.owner);
                self.execute_ai_seek_area(position, 0, flags, UNDEFINED_DIRECTION);
            }
            (SeekingArrowJustWatching, EventTimer) => {
                self.execute_ai_speech(crate::ai::AiSpeechAttempt {
                    remark: crate::ai::Remark::Arrow,
                    flags: crate::ai::SpeechFlags::MYTALK_1.bits(),
                })
            }
            (SeekingArrowJustWatching, EventMyTalk1) => {
                let position = self
                    .engine
                    .ai(self.owner, "arrow report position")
                    .seek_position;
                if !self.execute_ai_alert_soldiers(
                    position,
                    (SeekFlags::LOCATION_FIRST | SeekFlags::REPORT_OFFICER_AFTER).bits(),
                ) {
                    self.execute_ai_return_to_duty(DutyFlags::empty());
                }
            }
            (SeekingCombatAlertReactiontime, EventTimer) => {
                self.duty_set_state(AiState::Seeking, SeekingCombatAlert);
                let position = self
                    .engine
                    .ai(self.owner, "combat alert position")
                    .seek_position;
                self.duty_go_near(position, 50, GotoFlags::RUN);
            }
            (SeekingCombatAlert, EventReachPoint) => {
                let position = self
                    .engine
                    .ai(self.owner, "combat alert search")
                    .seek_position;
                self.execute_ai_seek_area(
                    position,
                    crate::parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                    SeekFlags::empty(),
                    UNDEFINED_DIRECTION,
                );
            }
            (SeekingKnightWatchingTowerGuard, EventTimer) => {
                let position = self
                    .engine
                    .ai(self.owner, "tower guard search")
                    .seek_position;
                self.execute_ai_seek_area(
                    position,
                    crate::parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                    SeekFlags::LOCATION_FIRST,
                    UNDEFINED_DIRECTION,
                );
            }
            (
                SeekingOfficerLookingForSoldiers1
                | SeekingOfficerLookingForSoldiers2
                | SeekingOfficerLookingForSoldiers3,
                EventTimer,
            ) => {
                let next = match substate {
                    SeekingOfficerLookingForSoldiers1 => SeekingOfficerLookingForSoldiers1Sidewards,
                    SeekingOfficerLookingForSoldiers2 => SeekingOfficerLookingForSoldiers2Sidewards,
                    _ => SeekingOfficerLookingForSoldiers3Sidewards,
                };
                self.duty_set_state(AiState::Seeking, next);
                let direction = if crate::sim_rng::u32(
                    self.sim,
                    crate::sim_rng::RngSite::OfficerSearchLook,
                    0..2,
                ) != 0
                {
                    LookDirection::RightLeft
                } else {
                    LookDirection::LeftRight
                };
                self.execute_ai_look_sidewards(direction);
            }
            (
                SeekingOfficerLookingForSoldiers1Sidewards
                | SeekingOfficerLookingForSoldiers2Sidewards,
                EventDone,
            ) => {
                let next = if substate == SeekingOfficerLookingForSoldiers1Sidewards {
                    SeekingOfficerLookingForSoldiers2
                } else {
                    SeekingOfficerLookingForSoldiers3
                };
                self.duty_set_state(AiState::Seeking, next);
                let direction = (self
                    .engine
                    .expect_entity(self.owner, "officer search direction")
                    .element_data()
                    .direction() as u16
                    + 5)
                    % 16;
                self.duty_face_direction(direction);
                self.engine.remaining_search_timer(self.owner, 30);
            }
            (SeekingOfficerLookingForSoldiers3Sidewards, EventDone) => {
                self.execute_ai_return_to_duty(DutyFlags::empty());
            }
            (MenacingPcInComa, EventTimer) => {
                let target = self
                    .engine
                    .ai(self.owner, "menacing target")
                    .primary_target
                    .expect("menacing requires target");
                let target = self
                    .engine
                    .expect_human_id_for_ai_handle(target.get(), "menacing target");
                let entity = self.engine.expect_entity(target, "menacing coma target");
                let there = entity.element_data().position();
                let here = self
                    .engine
                    .expect_entity(self.owner, "menacing owner")
                    .element_data()
                    .position();
                let distance = (there.x - here.x)
                    .abs()
                    .max(
                        ((there.y - here.y) * crate::position_interface::INVERSE_ASPECT_RATIO)
                            .abs(),
                    )
                    .max((there.z - here.z).abs());
                let in_coma = match entity {
                    Entity::Pc(pc) => {
                        self.engine.mission_domain.campaign.characters[pc
                            .pc
                            .campaign_description_index
                            .expect("menacing PC campaign identity")
                            as usize]
                            .status
                            .in_coma
                    }
                    _ => false,
                };
                if distance < 100.0 && entity.is_unconscious() && in_coma {
                    self.engine.remaining_search_timer(self.owner, 20);
                } else {
                    self.execute_ai_return_to_duty(DutyFlags::empty());
                }
            }
            _ => return Option::None,
        }
        Some(false)
    }
}
