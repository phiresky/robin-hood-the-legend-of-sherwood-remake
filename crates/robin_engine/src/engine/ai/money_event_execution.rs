use super::*;
use crate::ai::{
    AiEntityHandle, AiState, DutyFlags, EmoticonType, GotoFlags, MoneyFightOperation, Remark,
    ReportType, Stimulus, StimulusInfo, StimulusType, Substate,
};
use crate::parameters_ai;
use crate::sim_rng::SimulationContext;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_money_event(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        use StimulusType::*;
        use Substate::*;
        if !stimulus.stimulus_type.is_expected_class() {
            return Option::None;
        }
        let state = self.seek_enemy(owner).base.current_substate;
        if !matches!(
            state,
            WonderingMoneyReactiontime
                | WonderingApproachingMoney
                | WonderingRunningForMoney
                | WonderingTakingMoney
                | WonderingWatchingForMoreMoney
                | WonderingBrawlReactiontime
                | WonderingBrawlApproaching
                | WonderingBrawlHitting
                | WonderingBrawlGotHit
                | WonderingBrawlRecovering
                | WonderingApproachingToLoot
                | WonderingLooting
                | WonderingDrinkingAle
                | WonderingAleAway
        ) {
            return Option::None;
        }
        match (state, stimulus.stimulus_type) {
            (WonderingMoneyReactiontime, EventTimer) => {
                self.money_reaction_live(sim, assets, owner)
            }
            (WonderingApproachingMoney | WonderingRunningForMoney, EventTimer) => {
                self.money_race_live(sim, assets, owner)
            }
            (WonderingApproachingMoney | WonderingRunningForMoney, EventReachPoint) => {
                self.money_arrival_live(sim, assets, owner)
            }
            (WonderingTakingMoney, _) => {
                let next = self.take_nearest_live_money(owner).map(AiEntityHandle::new);
                self.seek_enemy_mut(owner).base.interesting_object = next;
                if next.is_some() {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Wondering,
                        WonderingMoneyReactiontime,
                    );
                    self.money_event_timer(owner, 1);
                } else {
                    self.watch_money_live(sim, assets, owner);
                }
            }
            (WonderingWatchingForMoreMoney, EventDone) => self.execute_money_fight(
                sim,
                assets,
                owner,
                MoneyFightOperation::CollectOrLootAfterLook,
            ),
            (WonderingDrinkingAle, EventDone) | (WonderingAleAway, EventTimer) => {
                if !self.seek_enemy(owner).other_seen_ale.is_empty() {
                    let next =
                        AiEntityHandle::new(self.seek_enemy_mut(owner).other_seen_ale.remove(0));
                    let ai = self.seek_enemy_mut(owner);
                    ai.base.interesting_object = Some(next);
                    ai.base.object_of_desire = Some(next);
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Wondering,
                        WonderingApproachingAle,
                    );
                    let target = self.money_object(owner);
                    self.duty_go_near(
                        sim,
                        assets,
                        owner,
                        self.live_ai_position(target),
                        parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                        GotoFlags::FIND_ACCESSIBLE,
                    );
                    let here = self.live_ai_position(owner);
                    self.seek_enemy_mut(owner).return_to_patrol_point = here;
                    self.money_event_timer(owner, 1);
                } else {
                    self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
                }
            }
            (WonderingBrawlReactiontime, EventTimer) => {
                self.clean_live_seen_money(owner);
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    WonderingBrawlApproaching,
                );
                self.seek_enemy_mut(owner)
                    .base
                    .set_emoticon(EmoticonType::Thunderstorm);
                self.money_say_live(sim, assets, owner, Remark::GoldBrawl);
                let position = self.live_ai_position(self.money_friend(owner));
                self.seek_enemy_mut(owner).base.seek_position = position;
                self.duty_go_near(
                    sim,
                    assets,
                    owner,
                    position,
                    parameters_ai::AI_HIT_DISTANCE,
                    GotoFlags::RUN,
                );
                self.money_event_timer(owner, 1);
            }
            (WonderingBrawlApproaching, EventTimer) => {
                let position = self.live_ai_position(self.money_friend(owner));
                let old = self.seek_enemy(owner).base.seek_position;
                if (old.x - position.x).abs().max((old.y - position.y).abs()) > 3.0 {
                    self.seek_enemy_mut(owner).base.seek_position = position;
                    self.duty_go_near(
                        sim,
                        assets,
                        owner,
                        position,
                        parameters_ai::AI_HIT_DISTANCE,
                        GotoFlags::RUN,
                    );
                    self.money_event_timer(owner, 1);
                }
            }
            (WonderingBrawlApproaching, EventReachPoint) => {
                self.brawl_arrival_live(sim, assets, owner)
            }
            (WonderingBrawlHitting, EventDone) => {
                let center = self.live_ai_position(owner);
                let count = self.world.npc_registry_ids.len();
                for index in 0..count {
                    let target = self.world.npc_registry_ids[index];
                    if matches!(self.get_entity(target), Some(Entity::Civilian(_)))
                        && self.live_ai_detects_180(assets, target, owner)
                    {
                        let mut panic = Stimulus::new(EventPanic);
                        panic.info = StimulusInfo::Position(center);
                        self.execute_ai_callback(sim, assets, target, &panic);
                    }
                }
                self.execute_maybe_officer_sees_me_fighting(sim, assets, owner);
                self.execute_money_fight(
                    sim,
                    assets,
                    owner,
                    MoneyFightOperation::FinishHitAfterOfficer,
                );
            }
            (WonderingBrawlGotHit, EventDone) => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    WonderingBrawlRecovering,
                );
                self.execute_maybe_officer_sees_me_fighting(sim, assets, owner);
                self.seek_enemy_mut(owner)
                    .base
                    .set_emoticon(EmoticonType::Thunderstorm);
                if let Some(friend) = self.seek_enemy(owner).base.friend_in_trouble {
                    let target = self.expect_human_id_for_ai_handle(friend.get(), "brawl attacker");
                    if self
                        .expect_entity(target, "brawl attacker")
                        .soldier_data()
                        .is_some()
                    {
                        assert_ne!(target, owner, "brawler cannot hit himself");
                        self.seek_enemy_mut(owner)
                            .money_fight_enemies
                            .push(friend.get());
                    }
                }
                if self.expect_entity(owner, "brawler posture").posture()
                    == crate::element::Posture::Lying
                {
                    self.stop_ai_owner(sim, assets, owner);
                    self.launch_element(
                        sim,
                        assets,
                        crate::sequence::SequenceElement::new(
                            1,
                            crate::element::Command::StandUp,
                            Some(owner),
                        ),
                    );
                } else {
                    self.execute_ai_callback(sim, assets, owner, &Stimulus::new(EventDone));
                }
            }
            (WonderingBrawlRecovering, EventDone) => {
                self.execute_money_fight(sim, assets, owner, MoneyFightOperation::RecoverBrawl)
            }
            (WonderingApproachingToLoot, EventReachPoint) => {
                self.loot_arrival_live(sim, assets, owner)
            }
            (WonderingLooting, EventDone) => self.loot_done_live(sim, assets, owner),
            _ => {}
        }

        Some(false)
    }

    fn money_event_timer(&mut self, owner: EntityId, duration: u32) {
        let frame = self.control.frame_counter;
        self.seek_enemy_mut(owner)
            .base
            .launch_timer(duration, frame);
    }
    fn money_say_live(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        remark: Remark,
    ) {
        self.execute_ai_speech(
            sim,
            assets,
            owner,
            crate::ai::AiSpeechAttempt { remark, flags: 0 },
        );
    }
    fn money_object(&self, owner: EntityId) -> EntityId {
        let handle = self
            .seek_enemy(owner)
            .base
            .interesting_object
            .expect("money or ale operation requires object");
        self.expect_entity_id_for_index(handle.get(), "money or ale object")
    }
    fn money_friend(&self, owner: EntityId) -> EntityId {
        let handle = self
            .seek_enemy(owner)
            .base
            .friend_in_trouble
            .expect("brawl requires partner");
        self.expect_human_id_for_ai_handle(handle.get(), "brawl partner")
    }
    fn money_body(&self, owner: EntityId) -> EntityId {
        let handle = self
            .seek_enemy(owner)
            .base
            .detected_body
            .expect("looting requires body");
        self.expect_human_id_for_ai_handle(handle.get(), "looting body")
    }
    fn money_soldier_at(&self, camp: crate::element::Camp, index: usize) -> EntityId {
        EntityId::Soldier(crate::entity_id::SoldierId(
            self.world.soldier_registry.camp(camp)[index],
        ))
    }
    fn money_reaction_live(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let actor = self.expect_entity(owner, "money reaction owner");
        let ai = self.seek_enemy(owner);
        let wants = i32::from(ai.base.blood_alcohol) > parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            || (actor.is_active()
                && !self.entity_data_in_building_sector(actor.element_data())
                && ai.profile(&assets.profile_manager).money > 0);
        let angry = if wants {
            let position = self.live_ai_position(self.money_object(owner));
            let camp = actor.camp();
            (0..self.world.soldier_registry.camp(camp).len()).any(|index| {
                let officer = self.money_soldier_at(camp, index);
                if !matches!(
                    self.seek_enemy(officer).base.current_substate,
                    Substate::WonderingOfficerSeeingBrawl
                        | Substate::WonderingOfficerApproachingBrawl
                        | Substate::WonderingOfficerFinishingBrawl
                ) {
                    return false;
                }
                let there = self.live_ai_position(officer);
                (position.x - there.x)
                    .abs()
                    .max((position.y - there.y).abs())
                    < 150.0
            })
        } else {
            false
        };
        if wants && !angry {
            if self.seek_enemy(owner).base.interesting_object.is_none()
                || !self
                    .expect_entity(self.money_object(owner), "reacted coin")
                    .is_active()
            {
                let next = self.take_nearest_live_money(owner).map(AiEntityHandle::new);
                self.seek_enemy_mut(owner).base.interesting_object = next;
            }
            self.money_say_live(sim, assets, owner, Remark::GoldYes);
            if self.seek_enemy(owner).base.interesting_object.is_some() {
                self.clean_live_seen_money(owner);
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    Substate::WonderingApproachingMoney,
                );
                let frame = self.control.frame_counter;
                self.seek_enemy_mut(owner).base.set_transient_emoticon(
                    EmoticonType::Sun,
                    20,
                    frame,
                );

                let target = self.money_object(owner);
                self.duty_go_near(
                    sim,
                    assets,
                    owner,
                    self.live_ai_position(target),
                    parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                    GotoFlags::FIND_ACCESSIBLE,
                );
                self.money_event_timer(owner, 5);
            } else {
                self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
            }
        } else {
            let frame = self.control.frame_counter;
            self.seek_enemy_mut(owner)
                .base
                .set_transient_emoticon(EmoticonType::Cloud, 50, frame);
            let remark = if self.expect_entity(owner, "coin refusal").is_vip() {
                Remark::VipGoldNo
            } else {
                Remark::GoldNo
            };
            self.money_say_live(sim, assets, owner, remark);
            self.seek_enemy_mut(owner).other_seen_money.clear();
            self.forget_ai_nearby_coins_live(owner);
            self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::KEEP_EMOTICON);
        }
    }
    fn money_race_live(&mut self, sim: &SimulationContext, assets: &LevelAssets, owner: EntityId) {
        let camp = self.expect_entity(owner, "money race camp").camp();
        let racing = (0..self.world.soldier_registry.camp(camp).len()).any(|index| {
            let other = self.money_soldier_at(camp, index);
            let state = self.seek_enemy(other).base.current_substate;
            other != owner
                && (state.is_take_money() || state.is_fight_for_money())
                && state != Substate::WonderingMoneyReactiontime
                && self.live_ai_detects_180(assets, owner, other)
        });
        if racing {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingRunningForMoney,
            );
            let target = self.money_object(owner);
            self.duty_go_near(
                sim,
                assets,
                owner,
                self.live_ai_position(target),
                parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                GotoFlags::RUN | GotoFlags::FIND_ACCESSIBLE,
            );
            if let Some(chief) = self.seek_enemy(owner).base.patrol_chief
                && self
                    .expect_entity(chief, "money patrol chief")
                    .soldier_data()
                    .is_some()
                && self.seek_enemy(chief).profile(&assets.profile_manager).rank
                    == crate::profiles::ProfileRank::Officer
                && self.live_ai_detects_180(assets, chief, owner)
            {
                let mut event = Stimulus::new(StimulusType::EventSeesBrawl);
                event.info = StimulusInfo::Human(AiEntityHandle::new(owner.index()));
                self.execute_ai_callback(sim, assets, chief, &event);
            }
        } else {
            self.money_event_timer(owner, 20);
        }
    }
    fn watch_money_live(&mut self, sim: &SimulationContext, assets: &LevelAssets, owner: EntityId) {
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Wondering,
            Substate::WonderingWatchingForMoreMoney,
        );
        self.execute_ai_look_sidewards(sim, assets, owner, crate::ai::LookDirection::LeftRight);
    }
    fn money_interaction_live(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: EntityId,
        command: crate::element::Command,
    ) {
        let mut sequence = crate::sequence::Sequence::new();
        sequence.append_element(crate::sequence::SequenceElement::new_interaction(
            1,
            command,
            Some(owner),
            Some(target),
        ));
        self.launch_sequence(sim, assets, sequence);
    }
    fn money_arrival_live(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let target = self.money_object(owner);
        let position = self.live_ai_position(target);
        let here = self.live_ai_position(owner);
        if self.expect_entity(target, "coin arrival").is_active()
            && (position.x - here.x).abs().max((position.y - here.y).abs()) < 25.0
        {
            let camp = self.expect_entity(owner, "coin recipient camp").camp();
            self.stop_ai_owner(sim, assets, owner);
            self.money_interaction_live(
                sim,
                assets,
                owner,
                self.money_object(owner),
                crate::element::Command::Take,
            );
            let stolen = crate::ai::StolenObject {
                object: self
                    .seek_enemy(owner)
                    .base
                    .interesting_object
                    .expect("taken coin"),
                thief: AiEntityHandle::new(owner.index()),
            };
            let count = self.world.soldier_registry.camp(camp).len();
            for index in 0..count {
                let other = self.money_soldier_at(camp, index);
                let state = self.seek_enemy(other).base.current_substate;
                if other != owner && (state.is_take_money() || state.is_fight_for_money()) {
                    let mut event = Stimulus::new(StimulusType::EventObjectAway);
                    event.info = StimulusInfo::Stolen(stolen);
                    self.execute_ai_callback(sim, assets, other, &event);
                }
            }
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingTakingMoney,
            );
        } else {
            self.watch_money_live(sim, assets, owner);
        }
    }
    fn brawl_arrival_live(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        if self.seek_enemy(owner).base.friend_in_trouble.is_none() {
            self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
            return;
        }
        let friend = self.money_friend(owner);
        if self
            .world
            .entities
            .expect_ai_controller(friend, format_args!("brawl partner state"))
            .current_state
            == AiState::Sleeping
        {
            self.seek_enemy_mut(owner)
                .money_fight_enemies
                .retain(|&handle| handle != friend.index());
            self.seek_enemy_mut(owner).base.friend_in_trouble = Option::None;
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingBrawlHitting,
            );
            self.execute_ai_callback(sim, assets, owner, &Stimulus::new(StimulusType::EventDone));
        } else {
            let a = self.live_ai_position(owner);
            let b = self.live_ai_position(friend);
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            if dx.hypot(dy) > parameters_ai::AI_HIT_DISTANCE as f32 + 3.0 {
                self.duty_go_near(
                    sim,
                    assets,
                    owner,
                    self.live_ai_position(friend),
                    parameters_ai::AI_HIT_DISTANCE,
                    GotoFlags::RUN,
                );
            } else {
                assert_ne!(owner, friend, "brawler cannot hit himself");
                self.stop_ai_owner(sim, assets, owner);
                self.money_interaction_live(
                    sim,
                    assets,
                    owner,
                    self.money_friend(owner),
                    crate::element::Command::HitCmd,
                );
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    Substate::WonderingBrawlHitting,
                );
            }
        }
    }
    fn loot_arrival_live(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let body = self.money_body(owner);
        let a = self
            .expect_entity(owner, "looter position")
            .element_data()
            .position();
        let b = self
            .expect_entity(body, "loot body position")
            .element_data()
            .position();
        let distance = (b.x - a.x)
            .abs()
            .max(((b.y - a.y) * crate::position_interface::INVERSE_ASPECT_RATIO).abs())
            .max((b.z - a.z).abs());
        if distance > 100.0 {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingLooting,
            );
            self.execute_ai_callback(sim, assets, owner, &Stimulus::new(StimulusType::EventDone));
        } else if self.expect_entity(body, "loot posture").posture()
            == crate::element::Posture::Tied
        {
            let position = self.live_ai_position(body);
            let report = &mut self.seek_enemy_mut(owner).base.my_reconnaissance_report;
            report.add_seen_body(body.index());
            report.update(ReportType::Body, position);
            self.duty_set_state(sim, assets, owner, AiState::Seeking, Substate::SeekingBody);
            self.execute_ai_callback(
                sim,
                assets,
                owner,
                &Stimulus::new(StimulusType::EventReachPoint),
            );
        } else {
            self.seek_enemy_mut(owner).old_money = self
                .expect_entity(owner, "looter money")
                .npc_data()
                .unwrap()
                .money as u16;
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingLooting,
            );
            self.stop_ai_owner(sim, assets, owner);
            self.money_interaction_live(
                sim,
                assets,
                owner,
                self.money_body(owner),
                crate::element::Command::SearchCmd,
            );
        }
    }
    fn loot_done_live(&mut self, sim: &SimulationContext, assets: &LevelAssets, owner: EntityId) {
        let gained = self
            .expect_entity(owner, "looter money")
            .npc_data()
            .unwrap()
            .money
            > u32::from(self.seek_enemy(owner).old_money);
        let frame = self.control.frame_counter;
        self.seek_enemy_mut(owner).base.set_transient_emoticon(
            if gained {
                EmoticonType::Sun
            } else {
                EmoticonType::Cloud
            },
            20,
            frame,
        );
        self.money_say_live(
            sim,
            assets,
            owner,
            if gained {
                Remark::SearchingSoldierGold
            } else {
                Remark::SearchingSoldierNothing
            },
        );
        while let Some(&next) = self.seek_enemy(owner).money_fight_victims.first() {
            let body = self.expect_human_id_for_ai_handle(next, "next looted victim");
            if !self.seek_enemy(body).base.looted_after_money_fight {
                break;
            }
            self.seek_enemy_mut(owner).money_fight_victims.remove(0);
        }
        if !self.seek_enemy(owner).money_fight_victims.is_empty() {
            let next = self.seek_enemy_mut(owner).money_fight_victims.remove(0);
            self.seek_enemy_mut(owner).base.detected_body = Some(AiEntityHandle::new(next));
            let body = self.money_body(owner);
            self.seek_enemy_mut(body).base.looted_after_money_fight = true;
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingApproachingToLoot,
            );
            self.duty_go_near(
                sim,
                assets,
                owner,
                self.live_ai_position(self.money_body(owner)),
                parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                GotoFlags::empty(),
            );
        } else {
            self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::KEEP_EMOTICON);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::seeking_event_execution::tests::fixture;
    use super::*;
    use crate::coordinates::WorldPoint3D;
    use crate::element::{ElementData, ElementKind, ElementProjectile, ObjectData, ProjectileData};
    use crate::element_kinds::ObjectType;

    fn object(engine: &mut EngineInner, owner: EntityId, x: f32, kind: ObjectType) -> EntityId {
        let mut element = ElementData::default();
        element.kind = ElementKind::ObjectProjectile;
        element.active = true;
        element.set_position(WorldPoint3D::new(x, 100.0, 0.0));
        element.set_sector(engine.live_ai_position(owner).sector);
        engine.add_test_entity(Entity::Projectile(ElementProjectile {
            element,
            object: ObjectData {
                object_type: kind,
                ..Default::default()
            },
            projectile: ProjectileData::default(),
        }))
    }
    fn set_state(engine: &mut EngineInner, owner: EntityId, substate: Substate) {
        let ai = engine.seek_enemy_mut(owner);
        ai.base.current_state = AiState::Wondering;
        ai.base.current_substate = substate;
    }
    fn event(
        engine: &mut EngineInner,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: StimulusType,
    ) {
        assert_eq!(
            engine.execute_ai_money_event(
                &crate::sim_rng::test_context(),
                assets,
                owner,
                &Stimulus::new(stimulus)
            ),
            Some(false)
        );
    }

    #[test]
    fn drinking_completion_and_ale_away_resume_the_next_remembered_bottle() {
        for (state, trigger) in [
            (Substate::WonderingDrinkingAle, StimulusType::EventDone),
            (Substate::WonderingAleAway, StimulusType::EventTimer),
        ] {
            let (mut engine, assets, owner, _) = fixture();
            let bottle = object(&mut engine, owner, 300.0, ObjectType::Ale);
            set_state(&mut engine, owner, state);
            engine.seek_enemy_mut(owner).base.blood_alcohol = 17;
            engine
                .seek_enemy_mut(owner)
                .other_seen_ale
                .push(bottle.index());
            event(&mut engine, &assets, owner, trigger);
            let ai = engine.seek_enemy(owner);
            assert_eq!(ai.base.current_substate, Substate::WonderingApproachingAle);
            assert_eq!(
                ai.base.interesting_object,
                Some(AiEntityHandle::new(bottle.index()))
            );
            assert_eq!(ai.base.object_of_desire, ai.base.interesting_object);
            assert!(ai.other_seen_ale.is_empty());
            assert_eq!(ai.base.blood_alcohol, 17);
            assert_eq!(ai.base.when_does_timer_ring, 101);
        }
    }

    #[test]
    fn taking_money_uses_the_nearest_live_coin_on_every_expected_event() {
        for trigger in [
            StimulusType::EventDone,
            StimulusType::EventTimer,
            StimulusType::CallYourTalk1,
        ] {
            let (mut engine, assets, owner, _) = fixture();
            let far = object(&mut engine, owner, 400.0, ObjectType::Coin);
            let near = object(&mut engine, owner, 140.0, ObjectType::Coin);
            let inactive = object(&mut engine, owner, 110.0, ObjectType::Coin);
            engine
                .get_entity_mut(inactive)
                .unwrap()
                .element_data_mut()
                .active = false;
            set_state(&mut engine, owner, Substate::WonderingTakingMoney);
            engine.seek_enemy_mut(owner).other_seen_money =
                vec![far.index(), inactive.index(), near.index()];
            event(&mut engine, &assets, owner, trigger);
            let ai = engine.seek_enemy(owner);
            assert_eq!(
                ai.base.current_substate,
                Substate::WonderingMoneyReactiontime
            );
            assert_eq!(
                ai.base.interesting_object,
                Some(AiEntityHandle::new(near.index()))
            );
            assert_eq!(ai.other_seen_money, vec![far.index()]);
            assert_eq!(ai.base.when_does_timer_ring, 101);
        }
    }

    #[test]
    fn projectile_coin_arrival_preserves_the_typed_take_target() {
        let (mut engine, assets, owner, _) = fixture();
        let coin = object(&mut engine, owner, 110.0, ObjectType::Coin);
        set_state(&mut engine, owner, Substate::WonderingApproachingMoney);
        engine.seek_enemy_mut(owner).base.interesting_object =
            Some(AiEntityHandle::new(coin.index()));
        event(&mut engine, &assets, owner, StimulusType::EventReachPoint);
        assert_eq!(
            engine.seek_enemy(owner).base.current_substate,
            Substate::WonderingTakingMoney
        );
        let take = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .find(|element| {
                element.owner == Some(owner) && element.command == crate::element::Command::Take
            })
            .expect("registered coin take");
        assert!(
            matches!(take.data, crate::sequence::SequenceElementData::Interaction { antagonist: Some(target) } if target == coin)
        );
    }

    #[test]
    fn money_race_without_a_visible_rival_only_rearms_poll() {
        let (mut engine, assets, owner, _) = fixture();
        let coin = object(&mut engine, owner, 300.0, ObjectType::Coin);
        set_state(&mut engine, owner, Substate::WonderingApproachingMoney);
        engine.seek_enemy_mut(owner).base.interesting_object =
            Some(AiEntityHandle::new(coin.index()));
        event(&mut engine, &assets, owner, StimulusType::EventTimer);
        assert_eq!(
            engine.seek_enemy(owner).base.current_substate,
            Substate::WonderingApproachingMoney
        );
        assert_eq!(engine.seek_enemy(owner).base.when_does_timer_ring, 120);
        assert!(
            engine
                .orders
                .sequence_manager
                .current_order_for_actor(&engine.world.entities, owner)
                .is_none()
        );
    }

    #[test]
    fn brawl_arrival_uses_live_distance_and_launches_hit_only_in_reach() {
        for x in [110.0, 500.0] {
            let (mut engine, assets, owner, friend) = fixture();
            engine
                .get_entity_mut(friend)
                .unwrap()
                .element_data_mut()
                .set_position(WorldPoint3D::new(x, 100.0, 0.0));
            set_state(&mut engine, owner, Substate::WonderingBrawlApproaching);
            engine.seek_enemy_mut(owner).base.friend_in_trouble =
                Some(AiEntityHandle::new(friend.index()));
            event(&mut engine, &assets, owner, StimulusType::EventReachPoint);
            assert_eq!(
                engine.seek_enemy(owner).base.current_substate,
                if x < 200.0 {
                    Substate::WonderingBrawlHitting
                } else {
                    Substate::WonderingBrawlApproaching
                }
            );
            if x < 200.0 {
                let hit = engine
                    .orders
                    .sequence_manager
                    .sequences_iter()
                    .flat_map(|sequence| sequence.elements.iter())
                    .find(|element| {
                        element.owner == Some(owner)
                            && element.command == crate::element::Command::HitCmd
                    })
                    .expect("registered brawl hit");
                assert!(
                    matches!(hit.data, crate::sequence::SequenceElementData::Interaction { antagonist: Some(target) } if target == friend)
                );
            }
        }
    }

    #[test]
    fn stationary_brawl_target_does_not_rearm_the_timer() {
        let (mut engine, assets, owner, friend) = fixture();
        set_state(&mut engine, owner, Substate::WonderingBrawlApproaching);
        let position = engine.live_ai_position(friend);
        let ai = engine.seek_enemy_mut(owner);
        ai.base.friend_in_trouble = Some(AiEntityHandle::new(friend.index()));
        ai.base.seek_position = position;
        ai.base.timer_is_running = false;
        event(&mut engine, &assets, owner, StimulusType::EventTimer);
        assert!(!engine.seek_enemy(owner).base.timer_is_running);
    }

    #[test]
    fn missing_or_sleeping_brawl_partners_finish_without_launching_a_hit() {
        for sleeping in [false, true] {
            let (mut engine, assets, owner, friend) = fixture();
            set_state(&mut engine, owner, Substate::WonderingBrawlApproaching);
            if sleeping {
                let ai = engine.seek_enemy_mut(friend);
                ai.base.current_state = AiState::Sleeping;
                ai.base.current_substate = Substate::SleepingUnconscious;
                engine
                    .get_entity_mut(friend)
                    .unwrap()
                    .human_data_mut()
                    .unwrap()
                    .unconscious = true;
                engine.seek_enemy_mut(owner).base.friend_in_trouble =
                    Some(AiEntityHandle::new(friend.index()));
                engine
                    .seek_enemy_mut(owner)
                    .money_fight_enemies
                    .push(friend.index());
            }
            event(&mut engine, &assets, owner, StimulusType::EventReachPoint);
            let ai = engine.seek_enemy(owner);
            assert!(!ai.money_fight_enemies.contains(&friend.index()));
            if sleeping {
                assert_eq!(
                    ai.base.current_substate,
                    Substate::WonderingWatchingForMoreMoney
                );
            } else {
                assert_eq!(ai.base.current_state, AiState::Default);
            }
        }
    }

    #[test]
    fn looting_reads_owner_money_and_marks_the_next_victim_before_approaching() {
        let (mut engine, assets, owner, body) = fixture();
        set_state(&mut engine, owner, Substate::WonderingLooting);
        engine
            .get_entity_mut(owner)
            .unwrap()
            .npc_data_mut()
            .unwrap()
            .money = 21;
        let ai = engine.seek_enemy_mut(owner);
        ai.old_money = 20;
        ai.money_fight_victims.push(body.index());
        event(&mut engine, &assets, owner, StimulusType::EventDone);
        assert!(engine.seek_enemy(body).base.looted_after_money_fight);
        let ai = engine.seek_enemy(owner);
        assert_eq!(
            ai.base.detected_body,
            Some(AiEntityHandle::new(body.index()))
        );
        assert_eq!(
            ai.base.current_substate,
            Substate::WonderingApproachingToLoot
        );
        assert_eq!(ai.base.current_emoticon_type, EmoticonType::Sun);
    }
}
