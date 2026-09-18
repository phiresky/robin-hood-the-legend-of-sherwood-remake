//! Enemy event dispatch reads the owner at each synchronous call boundary.

use super::*;
use crate::ai::{
    AiState, DutyFlags, EmoticonType, EnemyRecovery, MoneyFightOperation, Remark, Stimulus,
    StimulusInfo, Substate,
};
#[cfg(test)]
use crate::sim_rng::SimulationContext;

impl EngineInner {
    #[cfg(test)]
    pub(in crate::engine) fn execute_ai_enemy_event(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> bool {
        AiOwnerCtx::new(self, sim, assets, owner).execute_ai_enemy_event(stimulus)
    }
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_ai_enemy_event(&mut self, stimulus: &Stimulus) -> bool {
        if let Some(result) =
            self.engine
                .execute_ai_officer_rpc(self.sim, self.assets, self.owner, stimulus)
        {
            return result;
        }
        if self.execute_ai_combat_impact_event(stimulus) {
            return false;
        }
        if let Some(result) = self.execute_ai_default_event(stimulus) {
            return result;
        }
        if let Some(result) = self.execute_ai_remaining_search_event(stimulus) {
            return result;
        }
        if let Some(result) = self.execute_ai_remaining_wondering_event(stimulus) {
            return result;
        }
        if let Some(result) = self.execute_ai_enemy_fleeing_event(stimulus) {
            return result;
        }
        let event = stimulus.stimulus_type;
        let state = self.engine.observation_ai(self.owner).base.current_state;
        let substate = self.engine.observation_ai(self.owner).base.current_substate;
        let peaceful = matches!(
            state,
            AiState::Sleeping | AiState::Default | AiState::Wondering | AiState::Seeking
        );
        match event {
            StimulusType::EventView => {
                self.execute_ai_view_event(stimulus);
                return false;
            }
            StimulusType::EventHear if peaceful => {
                if self.dispatch_live_stimulus_to_patrol(stimulus) {
                    return false;
                }
                let StimulusInfo::Noise(noise) = stimulus.info else {
                    panic!("hearing event requires noise");
                };
                self.execute_ai_heard_noise(&noise);
                return false;
            }
            StimulusType::EventGetArrow if peaceful => {
                if self.dispatch_live_stimulus_to_patrol(stimulus) {
                    return false;
                }
                let StimulusInfo::Position(origin) = stimulus.info else {
                    panic!("arrow event requires origin");
                };
                self.execute_ai_received_arrow(origin);
                return false;
            }
            StimulusType::EventSeesObject if peaceful => {
                if self.dispatch_live_stimulus_to_patrol(stimulus) {
                    return false;
                }
                let StimulusInfo::Object(target) = stimulus.info else {
                    panic!("object sighting requires object");
                };
                self.execute_ai_seen_object(target.get());
                return false;
            }
            StimulusType::EventSeesShadow if state == AiState::Default => {
                if self.dispatch_live_stimulus_to_patrol(stimulus) {
                    return false;
                }
                let StimulusInfo::Position(position) = stimulus.info else {
                    panic!("shadow sighting requires position");
                };
                self.execute_ai_seen_shadow(position);
                return false;
            }
            StimulusType::CallLookThere => {
                if self.dispatch_live_stimulus_to_patrol(stimulus) {
                    return false;
                }
                let StimulusInfo::Hint(ref hint) = stimulus.info else {
                    panic!("look-there call requires hint");
                };
                self.execute_ai_look_there_reaction(hint.seek_point);
                return false;
            }
            StimulusType::CallTowerGuardAlert
                if matches!(state, AiState::Default | AiState::Wondering) =>
            {
                if self.dispatch_live_stimulus_to_patrol(stimulus) {
                    return false;
                }
                let StimulusInfo::Hint(ref hint) = stimulus.info else {
                    panic!("tower alert requires hint");
                };
                self.execute_ai_tower_alert_reaction(hint);
                return false;
            }
            StimulusType::CallTowerGuardCallsMe
                if matches!(state, AiState::Default | AiState::Wondering) =>
            {
                let StimulusInfo::Hint(ref hint) = stimulus.info else {
                    panic!("tower call requires hint");
                };
                self.execute_ai_tower_call_reaction(hint);
                return false;
            }
            StimulusType::EventSeesCharly
                if state == AiState::Seeking
                    || matches!(
                        substate,
                        Substate::DefaultLookingForCharly
                            | Substate::DefaultLookingSidewardsForCharly
                    ) =>
            {
                let StimulusInfo::Human(target) = stimulus.info else {
                    panic!("Charly sighting requires human");
                };
                self.execute_ai_seen_charly(target.get());
                return false;
            }
            StimulusType::CallCombatAlert => {
                assert_eq!(
                    self.engine
                        .observation_ai(self.owner)
                        .get_rank(&self.assets.profile_manager),
                    crate::profiles::ProfileRank::Soldier
                );
                if matches!(
                    state,
                    AiState::Default | AiState::Wondering | AiState::Seeking
                ) {
                    let StimulusInfo::Position(position) = stimulus.info else {
                        panic!("combat alert requires position");
                    };
                    self.execute_ai_combat_alert_reaction(position);
                    return true;
                }
                return state == AiState::Attacking;
            }
            _ => {}
        }
        let recovery = match event {
            StimulusType::EventFitAgain => Some(EnemyRecovery::FitAgain),
            StimulusType::EventWaspAway => Some(EnemyRecovery::WaspAway),
            StimulusType::EventNetAway => Some(EnemyRecovery::NetAway),
            StimulusType::EventStop => Some(EnemyRecovery::Stop),
            StimulusType::EventApple => {
                let StimulusInfo::Position(position) = stimulus.info else {
                    panic!("apple event requires position");
                };
                Some(EnemyRecovery::Apple { position })
            }
            StimulusType::EventStone
                if matches!(
                    state,
                    AiState::Sleeping | AiState::Default | AiState::Wondering
                ) =>
            {
                let StimulusInfo::Position(position) = stimulus.info else {
                    panic!("stone event requires position");
                };
                Some(EnemyRecovery::Stone { position })
            }
            _ => None,
        };
        if let Some(operation) = recovery {
            self.execute_ai_enemy_recovery(operation);
            return false;
        }
        match event {
            StimulusType::EventOutOfView => {
                return self.execute_ai_out_of_view(stimulus);
            }
            StimulusType::EventCouldntReachPoint => self.execute_ai_reachability_failure(),
            StimulusType::EventImpossible => {
                if substate == Substate::AttackingKillingSleepingEnemy {
                    self.execute_ai_get_battle_overview(0);
                } else {
                    self.execute_ai_callback(&Stimulus::new(StimulusType::EventDone));
                }
            }
            StimulusType::EventObjectAway => {
                let StimulusInfo::Stolen(stolen) = stimulus.info else {
                    panic!("object-away requires stolen object");
                };
                self.execute_money_fight(MoneyFightOperation::StolenMoney {
                    object: stolen.object,
                    thief: stolen.thief,
                });
            }
            StimulusType::CallCleanUpAfterBrawl
                if substate == Substate::WonderingSoldierLookingOfficerWhoFinishedBrawl =>
            {
                self.execute_money_fight(MoneyFightOperation::CleanUpAfterBrawl);
            }
            StimulusType::EventSeesBeggar if substate.is_seek_area() => {
                let StimulusInfo::Human(target) = stimulus.info else {
                    panic!("beggar sighting requires human");
                };
                let id = self
                    .engine
                    .expect_human_id_for_ai_handle(target.get(), "seen beggar");
                assert!(
                    !self
                        .engine
                        .observation_ai(self.owner)
                        .beggars_to_control
                        .contains(&target.get())
                );
                if self.engine.observation_ai(self.owner).beggar_to_examine != Some(target) {
                    let position = self.engine.live_ai_position(id);
                    self.engine
                        .observation_ai_mut(self.owner)
                        .beggars_to_control
                        .push(target.get());
                    self.engine
                        .observation_ai_mut(self.owner)
                        .positions_of_beggars_to_control
                        .push(position);
                }
                self.engine.delete_beggar_detectable_for_all_npc(id);
            }
            StimulusType::EventSeesBrawl if state == AiState::Default => {
                self.observation_stop();
                self.observation_say(Remark::OfficerSeesBrawl);
                let StimulusInfo::Human(friend) = stimulus.info else {
                    panic!("brawl sighting requires human");
                };
                self.engine
                    .observation_ai_mut(self.owner)
                    .base
                    .friend_in_trouble = Some(friend);
                let friend = self
                    .engine
                    .expect_human_id_for_ai_handle(friend.get(), "brawling friend");
                self.observation_face_entity(friend, false);
                self.engine.observation_emoticon(self.owner);
                let next = if self.engine.observation_ai(self.owner).base.blood_alcohol == 0 {
                    Substate::WonderingOfficerSeeingBrawl
                } else {
                    Substate::WonderingBrawlReactiontime
                };
                self.duty_set_state(AiState::Wondering, next);
                self.engine.observation_timer(self.owner, 30);
            }
            StimulusType::CallFinishBrawl
                if substate.is_take_money() || substate.is_fight_for_money() =>
            {
                self.observation_stop();
                let StimulusInfo::Human(officer) = stimulus.info else {
                    panic!("finish brawl requires officer");
                };
                let id = self
                    .engine
                    .expect_human_id_for_ai_handle(officer.get(), "brawl officer");
                self.observation_face_entity(id, false);
                self.engine
                    .observation_ai_mut(self.owner)
                    .base
                    .set_emoticon(EmoticonType::None);

                self.engine.observation_ai_mut(self.owner).base.antagonist = Some(officer);
                self.engine.forget_ai_nearby_coins_live(self.owner);
                self.duty_set_state(
                    AiState::Wondering,
                    Substate::WonderingSoldierLookingOfficerWhoFinishedBrawl,
                );
                let extra = crate::sim_rng::u32(
                    self.sim,
                    crate::sim_rng::RngSite::SoldierBrawlCooldown,
                    0..32,
                );
                self.engine.observation_timer(self.owner, 300 + extra);
            }
            StimulusType::EventAdversaryWeak | StimulusType::EventAfterCombatInjury
                if substate.is_real_swordfight() =>
            {
                if event == StimulusType::EventAfterCombatInjury {
                    self.observation_stop();
                }
                self.execute_reconsider_swordfight(event == StimulusType::EventAdversaryWeak);
                if event == StimulusType::EventAfterCombatInjury {
                    self.engine
                        .combat_insult_after_reconsider(self.sim, self.assets, self.owner);
                }
            }
            StimulusType::EventSwordStrike
                if matches!(
                    substate,
                    Substate::AttackingSwordfight
                        | Substate::AttackingSwordfightSpecialStrike
                        | Substate::AttackingMovingAroundOldEnemy
                        | Substate::AttackingApproachingNewEnemy
                ) =>
            {
                let StimulusInfo::Human(attacker) = stimulus.info else {
                    panic!("sword strike requires attacker");
                };
                self.engine.execute_ai_consider_to_begin_parade(
                    self.sim,
                    self.assets,
                    self.owner,
                    attacker.get(),
                );
            }
            StimulusType::EventGoodStrike | StimulusType::EventLethalStrike
                if substate == Substate::AttackingSwordfightSpecialStrike =>
            {
                let vip = self.engine.observation_ai(self.owner).is_vip;
                let remark = match (event == StimulusType::EventGoodStrike, vip) {
                    (true, true) => Remark::VipGoodStrikeCombat,
                    (true, false) => Remark::GoodStrikeCombat,
                    (false, true) => Remark::VipVictory,
                    (false, false) => Remark::KilledAdversary,
                };
                self.observation_say(remark);
            }
            StimulusType::EventReturnToDuty => self.execute_ai_return_to_duty(DutyFlags::empty()),
            _ => {}
        }
        false
    }

    fn execute_ai_view_event(&mut self, stimulus: &Stimulus) {
        let StimulusInfo::Human(target) = stimulus.info else {
            panic!("view event requires human");
        };
        let id = self
            .engine
            .expect_human_id_for_ai_handle(target.get(), "view target");
        let state = self.engine.observation_ai(self.owner).base.current_state;
        let substate = self.engine.observation_ai(self.owner).base.current_substate;
        let mut observe = false;
        match state {
            // Eyes are restored before the awakening timer leaves Sleeping.
            // Sightings during that delay do not interrupt recovery.
            AiState::Sleeping => {}
            AiState::Default | AiState::Wondering | AiState::Seeking => {
                observe = !self.dispatch_live_stimulus_to_patrol(stimulus)
            }
            AiState::Menacing => {
                observe = self
                    .engine
                    .observation_ai(self.owner)
                    .guarded_pc
                    .map(EntityId::Pc)
                    != Some(id)
            }
            AiState::Fleeing => {
                if !matches!(
                    substate,
                    Substate::FleeingMerryManRunToLeaveMap | Substate::FleeingRunForArrowReserves
                ) && (substate == Substate::FleeingHiding
                    || self
                        .engine
                        .observation_ai(self.owner)
                        .fleeing_seen_enemy_counter
                        < 20)
                {
                    self.engine
                        .observation_ai_mut(self.owner)
                        .fleeing_seen_enemy_counter += 1;
                    if self.engine.entity_data_in_building_sector(
                        self.engine
                            .expect_entity(self.owner, "fleeing observer")
                            .element_data(),
                    ) {
                        self.engine.dispatch_enemy_in_house_alert(
                            self.sim,
                            self.owner,
                            self.assets,
                        );
                    } else {
                        let position = self.engine.live_ai_position(id);
                        self.engine.execute_ai_panic(
                            self.sim,
                            self.assets,
                            self.owner,
                            Some(position),
                            crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
                            crate::ai::AlertLevel::Red,
                        );
                    }
                }
            }
            AiState::Attacking => match substate {
                Substate::AttackingReactiontimeTurning
                | Substate::AttackingReactiontime
                | Substate::AttackingReactiontimeRunning
                | Substate::AttackingOverviewLookLeft
                | Substate::AttackingOverviewLookRight
                | Substate::AttackingTooProudToAttackOverview => {
                    let enemies = &mut self.engine.observation_ai_mut(self.owner).list_them;
                    if !enemies.contains(&target.get()) {
                        enemies.push(target.get());
                    }
                }
                Substate::AttackingArcherWaitOnArcheryPath
                | Substate::AttackingArcherWaitOnBendPoint
                | Substate::AttackingArcherWaitOnArcheryPathBending => {
                    self.execute_ai_seen_enemy_as_archer(target.get());
                }
                Substate::AttackingApproachingSleepingEnemy
                | Substate::AttackingKillingSleepingEnemy => {
                    observe = !self
                        .engine
                        .expect_entity(id, "view unconscious gate")
                        .is_unconscious()
                }
                Substate::AttackingDoorFightDelay | Substate::AttackingDoorFightLeaving => {
                    if self.engine.entity_data_in_building_sector(
                        self.engine
                            .expect_entity(self.owner, "door observer")
                            .element_data(),
                    ) {
                        self.engine.dispatch_enemy_in_house_alert(
                            self.sim,
                            self.owner,
                            self.assets,
                        );
                    }
                }
                Substate::AttackingRiderChargingGettingDistance
                | Substate::AttackingRiderChargingReturning
                | Substate::AttackingRiderChargingApproachingBlindly => {
                    self.engine.reinitialize_live_ai_enemies(self.owner);
                    if !self.execute_ai_maybe_make_rider_attack() {
                        self.execute_battle_decisions();
                    }
                }
                _ => {}
            },
        }
        if observe {
            self.execute_ai_seen_enemy(target.get());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::test_support::{actors::make_test_ai_soldier, ensure_ordinary_sector};

    #[test]
    fn conscious_awakening_owner_ignores_view_without_interrupting_timer() {
        let mut engine = EngineInner::new();
        let sector = ensure_ordinary_sector(&mut engine, 1, 0);
        let mut entity = make_test_ai_soldier(crate::element::Camp::Lacklandists);
        entity.element_data_mut().set_sector(Some(sector));
        let owner = engine.add_test_entity(entity);
        let target = engine.add_test_entity(Entity::Pc(
            crate::engine::test_support::actors::unbound_pc(crate::element::Posture::Upright),
        ));
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        let sim = crate::sim_rng::test_context();
        let ai = engine.observation_ai_mut(owner);
        ai.base.current_state = AiState::Sleeping;
        ai.base.current_substate = Substate::SleepingAwakening;
        ai.base.launch_timer(100, 0);
        let before = engine.observation_ai(owner).clone();
        assert!(
            !engine
                .expect_entity(owner, "awakening owner")
                .is_unconscious()
        );
        let stimulus = Stimulus::with_human(StimulusType::EventView, target.index());
        assert!(engine.admit_ai_think_live(owner, &stimulus));
        assert!(!engine.execute_ai_enemy_event(&sim, &assets, owner, &stimulus));
        let after = engine.observation_ai(owner);
        assert_eq!(after.base.current_state, AiState::Sleeping);
        assert_eq!(after.base.current_substate, Substate::SleepingAwakening);
        assert!(after.base.timer_is_running);
        assert_eq!(after.base.when_does_timer_ring, 100);
        assert_eq!(after.base.antagonist, before.base.antagonist);
        assert_eq!(after.base.view_alert_status, before.base.view_alert_status);
        assert_eq!(after.list_them, before.list_them);
    }
}
