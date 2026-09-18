//! Looking and post-brawl conversations execute on live actors.

use super::*;
use crate::ai::{
    AiState, DutyFlags, EmoticonType, LookDirection, MoneyFightOperation, Remark, SpeechFlags,
    Stimulus, StimulusType, Substate,
};
use crate::engine::TickCtx;

impl EngineInner {
    fn remaining_wondering_timer(&mut self, owner: EntityId, frames: u32) {
        let frame = self.control.frame_counter;
        self.ai_mut(owner, "wondering timer")
            .launch_timer(frames, frame);
    }

    fn remaining_wondering_forget_coins(&mut self, owner: EntityId) {
        self.forget_ai_nearby_coins_live(owner);
    }

    pub(in crate::engine) fn forget_ai_nearby_coins_live(&mut self, owner: EntityId) {
        let list = crate::element::DetectableType::Object as usize;
        let mut index = 0;
        loop {
            let ai = self
                .expect_entity(owner, "forget coins owner")
                .ai_actor_data()
                .expect("forget coins owner data");
            let Some(detectable) = ai.detectable_lists[list].get(index) else {
                break;
            };
            let target = detectable
                .element
                .expect("object detectable requires element");
            let entity = self.expect_entity(target, "forget coin detectable");
            let object = entity
                .object_data()
                .expect("object detectable requires object");
            let there = entity.element_data().position();
            let here = self
                .expect_entity(owner, "forget coins position")
                .element_data()
                .position();
            let distance = (there.x - here.x)
                .abs()
                .max(((there.y - here.y) * crate::position_interface::INVERSE_ASPECT_RATIO).abs())
                .max((there.z - here.z).abs());
            if distance < 500.0 && object.object_type == crate::element_kinds::ObjectType::Coin {
                self.ai_actor_mut(owner, "forget nearby coin")
                    .detectable_lists[list]
                    .remove(index);
            } else {
                index += 1;
            }
        }
        self.enemy_ai_mut(owner, "brawl forget seen money")
            .other_seen_money
            .clear();
    }
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_ai_remaining_wondering_event(
        &mut self,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        use StimulusType::*;
        use Substate::*;
        let substate = self
            .engine
            .ai(self.owner, "wondering callback")
            .current_substate;
        let event = stimulus.stimulus_type;
        match (substate, event) {
            (WonderingWatching | WonderingWatchingTowerGuard, EventTimer)
            | (WonderingLooking3Sidewards, EventDone) => {
                self.execute_ai_return_to_duty(DutyFlags::empty());
            }
            (WonderingLooking1 | WonderingLooking2 | WonderingLooking3, EventTimer) => {
                let next = match substate {
                    WonderingLooking1 => WonderingLooking1Sidewards,
                    WonderingLooking2 => WonderingLooking2Sidewards,
                    _ => WonderingLooking3Sidewards,
                };
                self.duty_set_state(AiState::Wondering, next);
                let direction = if crate::sim_rng::u32(
                    self.sim,
                    crate::sim_rng::RngSite::EnemyWonderingLook,
                    0..2,
                ) != 0
                {
                    LookDirection::RightLeft
                } else {
                    LookDirection::LeftRight
                };
                self.execute_ai_look_sidewards(direction);
            }
            (WonderingLooking1Sidewards | WonderingLooking2Sidewards, EventDone) => {
                let next = if substate == WonderingLooking1Sidewards {
                    WonderingLooking2
                } else {
                    WonderingLooking3
                };
                self.duty_set_state(AiState::Wondering, next);
                let direction = (self
                    .engine
                    .expect_entity(self.owner, "wondering direction")
                    .element_data()
                    .direction() as u16
                    + 5)
                    % 16;
                self.duty_face_direction(direction);
                let delay = 30
                    + crate::sim_rng::u32(
                        self.sim,
                        crate::sim_rng::RngSite::EnemyWonderingLook,
                        0..8,
                    );
                self.engine.remaining_wondering_timer(self.owner, delay);
            }
            (WonderingAleReactiontime, EventTimer) => self.execute_ai_ale_reaction(),
            (WonderingApproachingAle, EventTimer | EventReachPoint) => {
                self.execute_ai_ale_approach(event == EventReachPoint)
            }
            (WonderingOfficerSeeingBrawl, EventTimer) => {
                self.duty_set_state(AiState::Wondering, WonderingOfficerApproachingBrawl);
                self.engine
                    .ai_mut(self.owner, "brawl officer emoticon")
                    .set_emoticon(EmoticonType::None);
                let target = self
                    .engine
                    .ai(self.owner, "brawl officer friend")
                    .friend_in_trouble
                    .expect("brawl officer requires friend");
                let target = self
                    .engine
                    .expect_human_id_for_ai_handle(target.get(), "brawl officer friend");
                self.duty_go_near(
                    self.engine.live_ai_position(target),
                    100,
                    crate::ai::GotoFlags::empty(),
                );
            }
            (WonderingOfficerApproachingBrawl, EventReachPoint | EventTimer) => {
                if event == EventReachPoint
                    && self
                        .engine
                        .ai(self.owner, "brawl officer speaking")
                        .current_remark
                        != Remark::TheSoundOfSilence
                {
                    self.engine.remaining_wondering_timer(self.owner, 50);
                } else {
                    self.execute_money_fight(MoneyFightOperation::FinishBrawl);
                }
            }
            (WonderingOfficerFinishingBrawl, EventMyTalk1 | CallYourTalk1 | CallYourTalk2) => {
                let (index, call) = match event {
                    EventMyTalk1 => (0, CallYourTalk1),
                    CallYourTalk1 => (1, CallYourTalk2),
                    _ => (2, CallYourTalk3),
                };
                let list = &self
                    .engine
                    .ai(self.owner, "brawl conversation participants")
                    .list_us;
                let target = list.get(index).copied();
                let first_npc = list.first().is_some_and(|handle| {
                    matches!(
                        self.engine
                            .expect_human_id_for_ai_handle(*handle, "brawl first participant"),
                        EntityId::Civilian(_) | EntityId::Soldier(_)
                    )
                });
                let answered = target.filter(|_| first_npc).is_some_and(|target| {
                    let target = self
                        .engine
                        .expect_human_id_for_ai_handle(target, "brawl conversation participant");
                    self.engine.execute_ai_callback(
                        TickCtx::new(self.sim, self.assets),
                        target,
                        &Stimulus::new(call),
                    )
                });
                if !answered {
                    self.execute_ai_callback(&Stimulus::new(call));
                }
            }
            (WonderingOfficerFinishingBrawl, CallYourTalk3) => {
                self.execute_ai_speech(crate::ai::AiSpeechAttempt {
                    remark: Remark::OfficerEndsConversation,
                    flags: SpeechFlags::MYTALK_2.bits(),
                })
            }
            (WonderingOfficerFinishingBrawl, EventMyTalk2 | EventTimer) => {
                let count = self
                    .engine
                    .ai(self.owner, "brawl dismissal count")
                    .list_us
                    .len();
                self.engine.remaining_wondering_forget_coins(self.owner);
                for index in 0..count {
                    let ai = self.engine.ai(self.owner, "brawl dismissal participant");
                    let target = ai.list_us[index];
                    if ai.antagonist.map(|handle| handle.get()) != Some(target) {
                        let target = self
                            .engine
                            .expect_human_id_for_ai_handle(target, "brawl dismissed participant");
                        if matches!(target, EntityId::Soldier(_)) {
                            self.engine.execute_ai_callback(
                                TickCtx::new(self.sim, self.assets),
                                target,
                                &Stimulus::new(EventReturnToDuty),
                            );
                        }
                    }
                }
                self.engine
                    .ai_mut(self.owner, "brawl dismissed list")
                    .list_us
                    .clear();
                let antagonist = self
                    .engine
                    .ai(self.owner, "brawl cleanup antagonist")
                    .antagonist;
                if let Some(target) = antagonist {
                    let target = self
                        .engine
                        .expect_human_id_for_ai_handle(target.get(), "brawl cleanup antagonist");
                    self.engine.execute_ai_callback(
                        TickCtx::new(self.sim, self.assets),
                        target,
                        &Stimulus::new(CallCleanUpAfterBrawl),
                    );
                    self.duty_set_state(AiState::Wondering, WonderingOfficerFinishingBrawlWaiting);
                    self.engine.remaining_wondering_timer(self.owner, 10);
                } else {
                    self.execute_ai_return_to_duty(DutyFlags::empty());
                }
            }
            (WonderingOfficerFinishingBrawlWaiting, EventTimer) => {
                let target = self
                    .engine
                    .ai(self.owner, "brawl cleanup waiter")
                    .antagonist
                    .expect("brawl cleanup requires antagonist");
                let target = self
                    .engine
                    .expect_human_id_for_ai_handle(target.get(), "brawl cleanup waiter");
                let state = self
                    .engine
                    .ai(target, "brawl cleanup state")
                    .current_substate;
                if matches!(
                    state,
                    WonderingApproachingBrawlVictim | WonderingAwakenBrawlVictim
                ) {
                    self.engine.remaining_wondering_timer(self.owner, 10);
                } else {
                    self.execute_ai_return_to_duty(DutyFlags::empty());
                }
            }
            (
                WonderingSoldierLookingOfficerWhoFinishedBrawl,
                CallYourTalk1 | CallYourTalk2 | CallYourTalk3,
            ) => {
                if event == CallYourTalk1 {
                    self.engine.ai.global.current_speech_variant = crate::sim_rng::u32(
                        self.sim,
                        crate::sim_rng::RngSite::EnemyBrawlExcuse,
                        0..3,
                    ) as u16;
                }
                self.engine
                    .ai_mut(self.owner, "brawl excuse emoticon")
                    .set_emoticon(EmoticonType::XMark);
                let flags = SpeechFlags::CYCLE_3_VARIANTS
                    | match event {
                        CallYourTalk1 => SpeechFlags::MYTALK_1,
                        CallYourTalk2 => SpeechFlags::MYTALK_2,
                        _ => SpeechFlags::MYTALK_3,
                    };
                self.execute_ai_speech(crate::ai::AiSpeechAttempt {
                    remark: Remark::BadExcuse,
                    flags: flags.bits(),
                });
                return Some(true);
            }
            (
                WonderingSoldierLookingOfficerWhoFinishedBrawl,
                EventMyTalk1 | EventMyTalk2 | EventMyTalk3,
            ) => {
                self.engine
                    .ai_mut(self.owner, "brawl excuse completion")
                    .set_emoticon(EmoticonType::None);

                if let Some(target) = self
                    .engine
                    .ai(self.owner, "brawl excuse officer")
                    .antagonist
                {
                    let target = self
                        .engine
                        .expect_human_id_for_ai_handle(target.get(), "brawl excuse officer");
                    let call = match event {
                        EventMyTalk1 => CallYourTalk1,
                        EventMyTalk2 => CallYourTalk2,
                        _ => CallYourTalk3,
                    };
                    self.engine.execute_ai_callback(
                        TickCtx::new(self.sim, self.assets),
                        target,
                        &Stimulus::new(call),
                    );
                }
            }
            (WonderingSoldierLookingOfficerWhoFinishedBrawl, EventTimer) => {
                self.engine.remaining_wondering_forget_coins(self.owner);
                self.execute_ai_return_to_duty(DutyFlags::empty());
            }
            (WonderingApproachingBrawlVictim, EventReachPoint) => {
                self.duty_set_state(AiState::Wondering, WonderingAwakenBrawlVictim);
                self.stop_ai_owner();
                let body = self
                    .engine
                    .ai(self.owner, "brawl victim wake target")
                    .detected_body
                    .expect("brawl victim required");
                let body = self
                    .engine
                    .expect_human_id_for_ai_handle(body.get(), "brawl victim wake target");
                self.engine.launch_element(
                    TickCtx::new(self.sim, self.assets),
                    crate::sequence::SequenceElement::new_interaction(
                        1,
                        crate::element::Command::WakeUp,
                        Some(self.owner),
                        Some(body),
                    ),
                );
            }
            (WonderingAwakenBrawlVictim, EventDone) => {
                self.execute_money_fight(MoneyFightOperation::AwakeNextVictim)
            }
            _ => return Option::None,
        }
        Some(false)
    }
}
