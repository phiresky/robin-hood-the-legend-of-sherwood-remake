//! Looking and post-brawl conversations execute on live actors.

use super::*;
use crate::ai::{
    AiState, DutyFlags, EmoticonType, LookDirection, MoneyFightOperation, Remark, SpeechFlags,
    Stimulus, StimulusType, Substate,
};
use crate::sim_rng::SimulationContext;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_remaining_wondering_event(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        use StimulusType::*;
        use Substate::*;
        let substate = self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("wondering callback"))
            .current_substate;
        let event = stimulus.stimulus_type;
        match (substate, event) {
            (WonderingWatching | WonderingWatchingTowerGuard, EventTimer)
            | (WonderingLooking3Sidewards, EventDone) => {
                self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
            }
            (WonderingLooking1 | WonderingLooking2 | WonderingLooking3, EventTimer) => {
                let next = match substate {
                    WonderingLooking1 => WonderingLooking1Sidewards,
                    WonderingLooking2 => WonderingLooking2Sidewards,
                    _ => WonderingLooking3Sidewards,
                };
                self.duty_set_state(sim, assets, owner, AiState::Wondering, next);
                let direction =
                    if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::EnemyWonderingLook, 0..2)
                        != 0
                    {
                        LookDirection::RightLeft
                    } else {
                        LookDirection::LeftRight
                    };
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("wondering look"))
                    .outbox
                    .actor
                    .look_sidewards = Some(direction);
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
            }
            (WonderingLooking1Sidewards | WonderingLooking2Sidewards, EventDone) => {
                let next = if substate == WonderingLooking1Sidewards {
                    WonderingLooking2
                } else {
                    WonderingLooking3
                };
                self.duty_set_state(sim, assets, owner, AiState::Wondering, next);
                let direction = (self
                    .expect_entity(owner, "wondering direction")
                    .element_data()
                    .direction() as u16
                    + 5)
                    % 16;
                self.duty_face_direction(sim, assets, owner, direction);
                let delay = 30
                    + crate::sim_rng::u32(sim, crate::sim_rng::RngSite::EnemyWonderingLook, 0..8);
                self.remaining_wondering_timer(owner, delay);
            }
            (WonderingAleReactiontime, EventTimer) => self.execute_ai_enemy_observation(
                sim,
                assets,
                owner,
                crate::ai::EnemyObservation::AleReaction,
            ),
            (WonderingApproachingAle, EventTimer | EventReachPoint) => self
                .execute_ai_enemy_observation(
                    sim,
                    assets,
                    owner,
                    crate::ai::EnemyObservation::AleApproach {
                        arrived: event == EventReachPoint,
                    },
                ),
            (WonderingOfficerSeeingBrawl, EventTimer) => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    WonderingOfficerApproachingBrawl,
                );
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("brawl officer emoticon"))
                    .set_emoticon(EmoticonType::None);
                let target = self
                    .world
                    .entities
                    .expect_ai_controller(owner, format_args!("brawl officer friend"))
                    .friend_in_trouble
                    .expect("brawl officer requires friend");
                let target =
                    self.expect_human_id_for_ai_handle(target.get(), "brawl officer friend");
                self.duty_go_near(
                    sim,
                    assets,
                    owner,
                    self.live_ai_position(target),
                    100,
                    crate::ai::GotoFlags::empty(),
                );
            }
            (WonderingOfficerApproachingBrawl, EventReachPoint | EventTimer) => {
                if event == EventReachPoint
                    && self
                        .world
                        .entities
                        .expect_ai_controller(owner, format_args!("brawl officer speaking"))
                        .current_remark
                        != Remark::TheSoundOfSilence
                {
                    self.remaining_wondering_timer(owner, 50);
                } else {
                    self.execute_money_fight(sim, assets, owner, MoneyFightOperation::FinishBrawl);
                }
            }
            (WonderingOfficerFinishingBrawl, EventMyTalk1 | CallYourTalk1 | CallYourTalk2) => {
                let (index, call) = match event {
                    EventMyTalk1 => (0, CallYourTalk1),
                    CallYourTalk1 => (1, CallYourTalk2),
                    _ => (2, CallYourTalk3),
                };
                let list = &self
                    .world
                    .entities
                    .expect_ai_controller(owner, format_args!("brawl conversation participants"))
                    .list_us;
                let target = list.get(index).copied();
                let first_npc = list.first().is_some_and(|handle| {
                    matches!(
                        self.expect_human_id_for_ai_handle(*handle, "brawl first participant"),
                        EntityId::Civilian(_) | EntityId::Soldier(_)
                    )
                });
                let answered = target.filter(|_| first_npc).is_some_and(|target| {
                    let target = self
                        .expect_human_id_for_ai_handle(target, "brawl conversation participant");
                    self.execute_ai_callback(sim, assets, target, &Stimulus::new(call))
                });
                if !answered {
                    self.execute_ai_callback(sim, assets, owner, &Stimulus::new(call));
                }
            }
            (WonderingOfficerFinishingBrawl, CallYourTalk3) => self.owner_work_speech(
                sim,
                assets,
                owner,
                crate::ai::AiSpeechAttempt {
                    remark: Remark::OfficerEndsConversation,
                    flags: SpeechFlags::MYTALK_2.bits(),
                },
            ),
            (WonderingOfficerFinishingBrawl, EventMyTalk2 | EventTimer) => {
                let count = self
                    .world
                    .entities
                    .expect_ai_controller(owner, format_args!("brawl dismissal count"))
                    .list_us
                    .len();
                self.remaining_wondering_forget_coins(sim, assets, owner);
                for index in 0..count {
                    let ai = self
                        .world
                        .entities
                        .expect_ai_controller(owner, format_args!("brawl dismissal participant"));
                    let target = ai.list_us[index];
                    if ai.antagonist.map(|handle| handle.get()) != Some(target) {
                        let target = self
                            .expect_human_id_for_ai_handle(target, "brawl dismissed participant");
                        if matches!(target, EntityId::Soldier(_)) {
                            self.execute_ai_callback(
                                sim,
                                assets,
                                target,
                                &Stimulus::new(EventReturnToDuty),
                            );
                        }
                    }
                }
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("brawl dismissed list"))
                    .list_us
                    .clear();
                let antagonist = self
                    .world
                    .entities
                    .expect_ai_controller(owner, format_args!("brawl cleanup antagonist"))
                    .antagonist;
                if let Some(target) = antagonist {
                    let target = self
                        .expect_human_id_for_ai_handle(target.get(), "brawl cleanup antagonist");
                    self.execute_ai_callback(
                        sim,
                        assets,
                        target,
                        &Stimulus::new(CallCleanUpAfterBrawl),
                    );
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Wondering,
                        WonderingOfficerFinishingBrawlWaiting,
                    );
                    self.remaining_wondering_timer(owner, 10);
                } else {
                    self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
                }
            }
            (WonderingOfficerFinishingBrawlWaiting, EventTimer) => {
                let target = self
                    .world
                    .entities
                    .expect_ai_controller(owner, format_args!("brawl cleanup waiter"))
                    .antagonist
                    .expect("brawl cleanup requires antagonist");
                let target =
                    self.expect_human_id_for_ai_handle(target.get(), "brawl cleanup waiter");
                let state = self
                    .world
                    .entities
                    .expect_ai_controller(target, format_args!("brawl cleanup state"))
                    .current_substate;
                if matches!(
                    state,
                    WonderingApproachingBrawlVictim | WonderingAwakenBrawlVictim
                ) {
                    self.remaining_wondering_timer(owner, 10);
                } else {
                    self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
                }
            }
            (
                WonderingSoldierLookingOfficerWhoFinishedBrawl,
                CallYourTalk1 | CallYourTalk2 | CallYourTalk3,
            ) => {
                if event == CallYourTalk1 {
                    self.ai.global.current_speech_variant =
                        crate::sim_rng::u32(sim, crate::sim_rng::RngSite::EnemyBrawlExcuse, 0..3)
                            as u16;
                }
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("brawl excuse emoticon"))
                    .set_emoticon(EmoticonType::XMark);
                let flags = SpeechFlags::CYCLE_3_VARIANTS
                    | match event {
                        CallYourTalk1 => SpeechFlags::MYTALK_1,
                        CallYourTalk2 => SpeechFlags::MYTALK_2,
                        _ => SpeechFlags::MYTALK_3,
                    };
                self.owner_work_speech(
                    sim,
                    assets,
                    owner,
                    crate::ai::AiSpeechAttempt {
                        remark: Remark::BadExcuse,
                        flags: flags.bits(),
                    },
                );
                return Some(true);
            }
            (
                WonderingSoldierLookingOfficerWhoFinishedBrawl,
                EventMyTalk1 | EventMyTalk2 | EventMyTalk3,
            ) => {
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("brawl excuse completion"))
                    .set_emoticon(EmoticonType::None);
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
                if let Some(target) = self
                    .world
                    .entities
                    .expect_ai_controller(owner, format_args!("brawl excuse officer"))
                    .antagonist
                {
                    let target =
                        self.expect_human_id_for_ai_handle(target.get(), "brawl excuse officer");
                    let call = match event {
                        EventMyTalk1 => CallYourTalk1,
                        EventMyTalk2 => CallYourTalk2,
                        _ => CallYourTalk3,
                    };
                    self.execute_ai_callback(sim, assets, target, &Stimulus::new(call));
                }
            }
            (WonderingSoldierLookingOfficerWhoFinishedBrawl, EventTimer) => {
                self.remaining_wondering_forget_coins(sim, assets, owner);
                self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
            }
            (WonderingApproachingBrawlVictim, EventReachPoint) => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    WonderingAwakenBrawlVictim,
                );
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("brawl victim wake stop"))
                    .stop_all();
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
                let body = self
                    .world
                    .entities
                    .expect_ai_controller(owner, format_args!("brawl victim wake target"))
                    .detected_body
                    .expect("brawl victim required");
                let body =
                    self.expect_human_id_for_ai_handle(body.get(), "brawl victim wake target");
                self.launch_element(crate::sequence::SequenceElement::new_interaction(
                    1,
                    crate::element::Command::WakeUp,
                    Some(owner),
                    Some(body),
                ));
            }
            (WonderingAwakenBrawlVictim, EventDone) => {
                self.execute_money_fight(sim, assets, owner, MoneyFightOperation::AwakeNextVictim)
            }
            _ => return Option::None,
        }
        Some(false)
    }

    fn remaining_wondering_timer(&mut self, owner: EntityId, frames: u32) {
        let frame = self.control.frame_counter;
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("wondering timer"))
            .launch_timer(frames, frame);
    }

    fn remaining_wondering_forget_coins(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
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
                self.world
                    .entities
                    .expect_ai_actor_data_mut(owner, format_args!("forget nearby coin"))
                    .detectable_lists[list]
                    .remove(index);
            } else {
                index += 1;
            }
        }
        self.world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("brawl forget seen money"))
            .other_seen_money
            .clear();
    }
}
