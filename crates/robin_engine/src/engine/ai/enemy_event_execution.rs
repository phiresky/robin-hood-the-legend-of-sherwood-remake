//! Enemy event dispatch reads the owner at each synchronous call boundary.

use super::*;
use crate::ai::{
    AiState, DutyFlags, EmoticonType, EnemyObservation, EnemyRecovery, MoneyFightOperation, Remark,
    Stimulus, StimulusInfo, Substate,
};
use crate::sim_rng::SimulationContext;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_enemy_event(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> bool {
        if let Some(result) = self.execute_ai_officer_rpc(sim, assets, owner, stimulus) {
            return result;
        }
        if self.execute_ai_combat_impact_event(sim, assets, owner, stimulus) {
            return false;
        }
        if let Some(result) = self.execute_ai_default_event(sim, assets, owner, stimulus) {
            return result;
        }
        if let Some(result) = self.execute_ai_remaining_search_event(sim, assets, owner, stimulus) {
            return result;
        }
        if let Some(result) =
            self.execute_ai_remaining_wondering_event(sim, assets, owner, stimulus)
        {
            return result;
        }
        if let Some(result) = self.execute_ai_enemy_fleeing_event(sim, assets, owner, stimulus) {
            return result;
        }
        let event = stimulus.stimulus_type;
        let state = self.observation_ai(owner).base.current_state;
        let substate = self.observation_ai(owner).base.current_substate;
        let peaceful = matches!(
            state,
            AiState::Sleeping | AiState::Default | AiState::Wondering | AiState::Seeking
        );
        let observation = match event {
            StimulusType::EventView => {
                self.execute_ai_view_event(sim, assets, owner, stimulus);
                return false;
            }
            StimulusType::EventHear if peaceful => {
                if self.dispatch_live_stimulus_to_patrol(sim, assets, owner, stimulus) {
                    return false;
                }
                let StimulusInfo::Noise(noise) = stimulus.info else {
                    panic!("hearing event requires noise");
                };
                Some(EnemyObservation::Noise { noise })
            }
            StimulusType::EventGetArrow if peaceful => {
                if self.dispatch_live_stimulus_to_patrol(sim, assets, owner, stimulus) {
                    return false;
                }
                let StimulusInfo::Position(origin) = stimulus.info else {
                    panic!("arrow event requires origin");
                };
                Some(EnemyObservation::Arrow { origin })
            }
            StimulusType::EventSeesObject if peaceful => {
                if self.dispatch_live_stimulus_to_patrol(sim, assets, owner, stimulus) {
                    return false;
                }
                let StimulusInfo::Object(target) = stimulus.info else {
                    panic!("object sighting requires object");
                };
                Some(EnemyObservation::Object {
                    target: target.get(),
                })
            }
            StimulusType::EventSeesShadow if state == AiState::Default => {
                if self.dispatch_live_stimulus_to_patrol(sim, assets, owner, stimulus) {
                    return false;
                }
                let StimulusInfo::Position(position) = stimulus.info else {
                    panic!("shadow sighting requires position");
                };
                Some(EnemyObservation::Shadow { position })
            }
            StimulusType::CallLookThere => {
                if self.dispatch_live_stimulus_to_patrol(sim, assets, owner, stimulus) {
                    return false;
                }
                let StimulusInfo::Hint(ref hint) = stimulus.info else {
                    panic!("look-there call requires hint");
                };
                Some(EnemyObservation::LookThere {
                    position: hint.seek_point,
                })
            }
            StimulusType::CallTowerGuardAlert
                if matches!(state, AiState::Default | AiState::Wondering) =>
            {
                if self.dispatch_live_stimulus_to_patrol(sim, assets, owner, stimulus) {
                    return false;
                }
                let StimulusInfo::Hint(ref hint) = stimulus.info else {
                    panic!("tower alert requires hint");
                };
                Some(EnemyObservation::TowerGuardAlert { hint: hint.clone() })
            }
            StimulusType::CallTowerGuardCallsMe
                if matches!(state, AiState::Default | AiState::Wondering) =>
            {
                let StimulusInfo::Hint(ref hint) = stimulus.info else {
                    panic!("tower call requires hint");
                };
                Some(EnemyObservation::TowerGuardCalls { hint: hint.clone() })
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
                Some(EnemyObservation::Charly {
                    target: target.get(),
                })
            }
            StimulusType::CallCombatAlert => {
                assert_eq!(
                    self.observation_ai(owner).get_rank(),
                    crate::profiles::ProfileRank::Soldier
                );
                if matches!(
                    state,
                    AiState::Default | AiState::Wondering | AiState::Seeking
                ) {
                    let StimulusInfo::Position(position) = stimulus.info else {
                        panic!("combat alert requires position");
                    };
                    self.execute_ai_enemy_observation(
                        sim,
                        assets,
                        owner,
                        EnemyObservation::CombatAlert { position },
                    );
                    return true;
                }
                return state == AiState::Attacking;
            }
            _ => None,
        };
        if let Some(operation) = observation {
            self.execute_ai_enemy_observation(sim, assets, owner, operation);
            return false;
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
            self.execute_ai_enemy_recovery(sim, assets, owner, operation);
            return false;
        }
        match event {
            StimulusType::EventOutOfView => {
                return self.execute_ai_out_of_view(sim, assets, owner, stimulus);
            }
            StimulusType::EventCouldntReachPoint => {
                self.execute_ai_reachability_failure(sim, assets, owner)
            }
            StimulusType::EventImpossible => {
                if substate == Substate::AttackingKillingSleepingEnemy {
                    self.execute_ai_get_battle_overview(sim, assets, owner, 0);
                } else {
                    self.execute_ai_callback(
                        sim,
                        assets,
                        owner,
                        &Stimulus::new(StimulusType::EventDone),
                    );
                }
            }
            StimulusType::EventObjectAway => {
                let StimulusInfo::Stolen(stolen) = stimulus.info else {
                    panic!("object-away requires stolen object");
                };
                self.execute_money_fight(
                    sim,
                    assets,
                    owner,
                    MoneyFightOperation::StolenMoney {
                        object: stolen.object,
                        thief: stolen.thief,
                    },
                );
            }
            StimulusType::CallCleanUpAfterBrawl
                if substate == Substate::WonderingSoldierLookingOfficerWhoFinishedBrawl =>
            {
                self.execute_money_fight(
                    sim,
                    assets,
                    owner,
                    MoneyFightOperation::CleanUpAfterBrawl,
                );
            }
            StimulusType::EventSeesBeggar if substate.is_seek_area() => {
                let StimulusInfo::Human(target) = stimulus.info else {
                    panic!("beggar sighting requires human");
                };
                let id = self.expect_human_id_for_ai_handle(target.get(), "seen beggar");
                assert!(
                    !self
                        .observation_ai(owner)
                        .beggars_to_control
                        .contains(&target.get())
                );
                if self.observation_ai(owner).beggar_to_examine != Some(target) {
                    let position = self.live_ai_position(id);
                    self.observation_ai_mut(owner)
                        .beggars_to_control
                        .push(target.get());
                    self.observation_ai_mut(owner)
                        .positions_of_beggars_to_control
                        .push(position);
                }
                self.delete_beggar_detectable_for_all_npc(id);
            }
            StimulusType::EventSeesBrawl if state == AiState::Default => {
                self.observation_stop(sim, assets, owner);
                self.observation_say(sim, assets, owner, Remark::OfficerSeesBrawl);
                let StimulusInfo::Human(friend) = stimulus.info else {
                    panic!("brawl sighting requires human");
                };
                self.observation_ai_mut(owner).base.friend_in_trouble = Some(friend);
                let friend = self.expect_human_id_for_ai_handle(friend.get(), "brawling friend");
                self.observation_face_entity(sim, assets, owner, friend, false);
                self.observation_emoticon(sim, assets, owner);
                let next = if self.observation_ai(owner).base.blood_alcohol == 0 {
                    Substate::WonderingOfficerSeeingBrawl
                } else {
                    Substate::WonderingBrawlReactiontime
                };
                self.duty_set_state(sim, assets, owner, AiState::Wondering, next);
                self.observation_timer(owner, 30);
            }
            StimulusType::CallFinishBrawl
                if substate.is_take_money() || substate.is_fight_for_money() =>
            {
                self.observation_stop(sim, assets, owner);
                let StimulusInfo::Human(officer) = stimulus.info else {
                    panic!("finish brawl requires officer");
                };
                let id = self.expect_human_id_for_ai_handle(officer.get(), "brawl officer");
                self.observation_face_entity(sim, assets, owner, id, false);
                self.observation_ai_mut(owner)
                    .base
                    .set_emoticon(EmoticonType::None);
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
                self.observation_ai_mut(owner).base.antagonist = Some(officer);
                self.forget_ai_nearby_coins_live(owner);
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    Substate::WonderingSoldierLookingOfficerWhoFinishedBrawl,
                );
                let extra =
                    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::SoldierBrawlCooldown, 0..32);
                self.observation_timer(owner, 300 + extra);
            }
            StimulusType::EventAdversaryWeak | StimulusType::EventAfterCombatInjury
                if substate.is_real_swordfight() =>
            {
                if event == StimulusType::EventAfterCombatInjury {
                    self.observation_stop(sim, assets, owner);
                }
                self.execute_reconsider_swordfight(
                    sim,
                    assets,
                    owner,
                    event == StimulusType::EventAdversaryWeak,
                );
                if event == StimulusType::EventAfterCombatInjury {
                    self.observation_ai_mut(owner).finish_after_combat_injury();
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
                self.observation_ai_mut(owner)
                    .base
                    .outbox
                    .reentrant
                    .owner_work
                    .push(crate::ai::AiOwnerWork::ConsiderToBeginParade {
                        attacker: attacker.get(),
                    });
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
            }
            StimulusType::EventGoodStrike | StimulusType::EventLethalStrike
                if substate == Substate::AttackingSwordfightSpecialStrike =>
            {
                let vip = self.observation_ai(owner).is_vip;
                let remark = match (event == StimulusType::EventGoodStrike, vip) {
                    (true, true) => Remark::VipGoodStrikeCombat,
                    (true, false) => Remark::GoodStrikeCombat,
                    (false, true) => Remark::VipVictory,
                    (false, false) => Remark::KilledAdversary,
                };
                self.observation_say(sim, assets, owner, remark);
            }
            StimulusType::EventReturnToDuty => {
                self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty())
            }
            _ => {}
        }
        false
    }

    fn execute_ai_view_event(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) {
        let StimulusInfo::Human(target) = stimulus.info else {
            panic!("view event requires human");
        };
        let id = self.expect_human_id_for_ai_handle(target.get(), "view target");
        let state = self.observation_ai(owner).base.current_state;
        let substate = self.observation_ai(owner).base.current_substate;
        let mut observe = false;
        match state {
            AiState::Sleeping => panic!("sleeping owner received view event"),
            AiState::Default | AiState::Wondering | AiState::Seeking => {
                observe = !self.dispatch_live_stimulus_to_patrol(sim, assets, owner, stimulus)
            }
            AiState::Menacing => {
                observe = self.observation_ai(owner).guarded_pc.map(EntityId::Pc) != Some(id)
            }
            AiState::Fleeing => {
                if !matches!(
                    substate,
                    Substate::FleeingMerryManRunToLeaveMap | Substate::FleeingRunForArrowReserves
                ) && (substate == Substate::FleeingHiding
                    || self.observation_ai(owner).fleeing_seen_enemy_counter < 20)
                {
                    self.observation_ai_mut(owner).fleeing_seen_enemy_counter += 1;
                    if self.entity_data_in_building_sector(
                        self.expect_entity(owner, "fleeing observer").element_data(),
                    ) {
                        self.dispatch_enemy_in_house_alert(sim, owner, assets);
                    } else {
                        let position = self.live_ai_position(id);
                        self.observation_ai_mut(owner).panic_from_position(
                            position,
                            crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
                        );
                        self.drain_direct_ai_owner_boundary(sim, owner, assets);
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
                    self.observation_ai_mut(owner).list_them.push(target.get());
                }
                Substate::AttackingArcherWaitOnArcheryPath
                | Substate::AttackingArcherWaitOnBendPoint
                | Substate::AttackingArcherWaitOnArcheryPathBending => {
                    self.execute_ai_enemy_observation(
                        sim,
                        assets,
                        owner,
                        EnemyObservation::ArcherEnemy {
                            target: target.get(),
                        },
                    );
                }
                Substate::AttackingApproachingSleepingEnemy
                | Substate::AttackingKillingSleepingEnemy => {
                    observe = !self
                        .expect_entity(id, "view unconscious gate")
                        .is_unconscious()
                }
                Substate::AttackingDoorFightDelay | Substate::AttackingDoorFightLeaving => {
                    if self.entity_data_in_building_sector(
                        self.expect_entity(owner, "door observer").element_data(),
                    ) {
                        self.dispatch_enemy_in_house_alert(sim, owner, assets);
                    }
                }
                Substate::AttackingRiderChargingGettingDistance
                | Substate::AttackingRiderChargingReturning
                | Substate::AttackingRiderChargingApproachingBlindly => {
                    self.reinitialize_live_ai_enemies(owner);
                    if !self.execute_ai_maybe_make_rider_attack(sim, assets, owner) {
                        self.execute_battle_decisions(sim, assets, owner);
                    }
                }
                _ => {}
            },
        }
        if observe {
            self.execute_ai_enemy_observation(
                sim,
                assets,
                owner,
                EnemyObservation::Enemy {
                    target: target.get(),
                },
            );
        }
    }
}
