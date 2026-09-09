//! Exact simulation hooks for mission achievement evaluation.

use crate::{
    achievement::{
        AchievementBuildingId, AchievementDeathCause, AchievementEntitySnapshot,
        AchievementProgressSnapshot,
    },
    element::{Entity, EntityId, Human as _},
    pc_status::Skill,
};

use super::EngineInner;

/// Detailed live XP for one exact campaign-backed player character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PcExperienceSnapshot {
    pub entity: EntityId,
    pub hand_to_hand: Skill,
    pub bow: Skill,
}

impl EngineInner {
    /// Freeze campaign-only deeds at a real progression victory. Practice
    /// cannot rewrite a campaign's losses, work history, or story milestones.
    pub(super) fn evaluate_campaign_deeds(&mut self, assets: &super::LevelAssets) {
        use crate::achievement::{AchievementEvaluation as E, AchievementId as A};
        use crate::character_kind::CharacterKind as K;
        let campaign = &mut self.mission_domain.campaign;
        let Some(idx) = campaign.current_mission_idx else {
            return;
        };
        if campaign.history_replay_mission_idx.is_some() {
            return;
        }
        let profile = campaign.missions[idx].profile(&assets.profile_manager);
        let ransom_sent = profile.mission_name == "H10_Yor_VL";
        // Unlike a 100% fraction of a demo's tiny catalogue, H12 is the
        // Original full-campaign completion boundary.
        let full_complete = campaign.achievement_envelope_complete(&assets.profile_manager);
        for (_, entity) in self.world.entities.occupied() {
            let Entity::Pc(pc) = entity else {
                continue;
            };
            if pc.pc.mission_role != crate::human_control::MissionRole::PlayerParty {
                continue;
            }
            let character =
                pc.pc
                    .campaign_description_index
                    .expect("party member has no campaign identity") as usize;
            assert_eq!(
                campaign.characters[character].character_profile_idx,
                Some(pc.pc.profile_index),
                "party member campaign identity mismatch"
            );
            if !campaign.gang_indices.contains(&character) {
                continue;
            }
            match pc.pc.kind {
                Some(K::LittleJohn) => {
                    campaign.deeds.companion_victories.insert(0);
                }
                Some(K::FriarTuck) => {
                    campaign.deeds.companion_victories.insert(1);
                }
                Some(K::Stutely) => {
                    campaign.deeds.companion_victories.insert(2);
                }
                Some(K::WillScarlet) => {
                    campaign.deeds.companion_victories.insert(3);
                }
                Some(K::LadyMarianne) => {
                    campaign.deeds.companion_victories.insert(4);
                }
                Some(K::MerryManA | K::MerryManB | K::MerryManC) => {
                    if self.mission_domain.achievements.has_contributed(character) {
                        campaign.deeds.contributing_veterans.insert(character);
                    }
                }
                _ => {}
            }
        }
        let history_eligible = campaign
            .missions
            .iter()
            .flat_map(|m| m.attempt_history().attempts())
            .filter(|a| {
                a.kind() == crate::campaign_history::MissionAttemptKind::Campaign
                    && a.outcome() == crate::campaign_history::MissionAttemptOutcome::Won
            })
            .all(|a| {
                a.achievement_attestation()
                    .is_some_and(|attestation| attestation.decision().may_persist())
            });
        let complete_evidence = campaign.deeds.complete_evidence && history_eligible;
        let workers = campaign
            .deeds
            .workers
            .intersection(&campaign.deeds.contributing_veterans)
            .count();
        for (id, earned) in [
            (A::ALegendIsBorn, full_complete),
            (A::ForKingRichard, ransom_sent),
            (
                A::WholeMerryCompany,
                full_complete && campaign.deeds.companion_victories.len() == 5,
            ),
            (
                A::NoEmptyPlaces,
                full_complete && campaign.deeds.lost_members.is_empty(),
            ),
            (A::ManyHands, full_complete && workers >= 3),
        ] {
            if self
                .mission_domain
                .achievements
                .verifiable_achievements()
                .contains(id)
            {
                self.mission_domain
                    .achievements
                    .record_evaluation(
                        id,
                        if !complete_evidence {
                            E::Unverifiable
                        } else if earned {
                            E::Earned
                        } else {
                            E::Failed
                        },
                    )
                    .expect("campaign achievement finalized too early");
            }
        }
    }

    pub(super) fn record_achievement_tactical_effect(&mut self, actor: EntityId, target: EntityId) {
        let hostile = self.world.entities.get(target).is_some_and(|entity| {
            entity.npc_data().is_some() && self.is_hostile_to_player_camp(entity.camp())
        });
        if hostile {
            self.mission_domain
                .achievements
                .record_qa_success(actor, target);
            self.record_achievement_contribution(actor);
        }
    }

    pub(super) fn record_achievement_contribution(&mut self, actor: EntityId) {
        let Some(pc) = self.world.entities.get(actor).and_then(Entity::pc_data) else {
            return;
        };
        if pc.mission_role != crate::human_control::MissionRole::PlayerParty {
            return;
        }
        if self.mission_domain.campaign.current_mission_idx.is_none() {
            return;
        }
        let index = self
            .pc_description_index_for_pc_data(pc)
            .expect("contributing party member has no campaign identity");
        self.mission_domain.achievements.record_contribution(index);
    }
    /// Capture the baseline after startup scripts and Sherwood production
    /// setup have settled. This is deliberately later than level parsing:
    /// scripted setup deaths must not be attributed to the player.
    pub(super) fn initialize_achievement_tracking(&mut self, assets: &super::LevelAssets) {
        let baseline = self
            .world
            .entities
            .occupied()
            .filter_map(|(id, entity)| {
                let dead = match entity {
                    Entity::Soldier(soldier) => soldier.life_points() <= 0,
                    Entity::Civilian(civilian) => civilian.life_points() <= 0,
                    _ => return None,
                };
                Some((id, dead))
            })
            .collect::<Vec<_>>();
        self.mission_domain
            .achievements
            .initialize_mission_baseline(self.control.frame_counter, baseline);
        let banners = self
            .mission_domain
            .campaign
            .current_mission_idx
            .and_then(|idx| {
                let campaign = &self.mission_domain.campaign;
                let mission = &campaign.missions[idx];
                let profile = mission.profile(&assets.profile_manager);
                if !mission.requires_blazons(&assets.profile_manager) {
                    return None;
                }
                let total = u32::from(profile.number_of_blazons_to_win)
                    .checked_sub(u32::from(profile.number_of_blazons_to_be_collected))
                    .expect("mission banner requirement underflows");
                (mission.requires_blazons(&assets.profile_manager) && total > 0).then(|| {
                    (
                        campaign
                            .deeds
                            .purchased_banners
                            .get(&profile.id)
                            .copied()
                            .unwrap_or(0),
                        total,
                    )
                })
            });
        self.mission_domain.achievements.configure_banners(banners);
        self.refresh_achievement_progress(assets);
    }

    /// Refresh all arrangement-derived progress from exact live entity and
    /// sector state. The scan order is irrelevant; the tracker uses ordered
    /// identity sets/maps and is included in replay/rollback state.
    pub(super) fn refresh_achievement_progress(&mut self, assets: &super::LevelAssets) {
        let party = self
            .world
            .entities
            .occupied()
            .filter_map(|(id, e)| {
                let pc = e.pc_data()?;
                (pc.mission_role == crate::human_control::MissionRole::PlayerParty).then_some((
                    id,
                    i32::from(pc.life_points),
                    pc.kind,
                ))
            })
            .collect::<Vec<_>>();
        for &(id, hp, _) in &party {
            self.mission_domain.achievements.record_party_health(id, hp);
        }
        let alive = self
            .world
            .entities
            .occupied()
            .filter(|(_, e)| {
                matches!(e, Entity::Soldier(s) if !s.is_out_of_order()) && e.element_data().active
            })
            .map(|(id, _)| id)
            .collect();
        let pursuit = self
            .world
            .entities
            .occupied()
            .filter_map(|(id, e)| {
                if !matches!(e, Entity::Soldier(_)) || !self.is_hostile_to_player_camp(e.camp()) {
                    return None;
                }
                let ai = e.enemy_ai()?;
                let party_target = ai.base.primary_target.is_some_and(|target| {
                    party.iter().any(|(pc, _, _)| pc.index() == target.get())
                });
                let still_searching = self.mission_domain.achievements.is_tracked_pursuer(id)
                    && matches!(
                        ai.base.current_state,
                        crate::ai::AiState::Seeking | crate::ai::AiState::Wondering
                    );
                ((party_target
                    && matches!(
                        ai.base.current_state,
                        crate::ai::AiState::Attacking
                            | crate::ai::AiState::Seeking
                            | crate::ai::AiState::Menacing
                    ))
                    || still_searching)
                    .then_some(id)
            })
            .collect();
        if party.iter().any(|(id, _, _)| {
            self.world
                .entities
                .get(*id)
                .is_some_and(|e| e.element_data().active)
        }) {
            self.mission_domain
                .achievements
                .refresh_pursuit(pursuit, alive);
        }
        let generic_only = !party.is_empty()
            && party.iter().all(|(_, _, kind)| {
                matches!(
                    kind,
                    Some(
                        crate::character_kind::CharacterKind::MerryManA
                            | crate::character_kind::CharacterKind::MerryManB
                            | crate::character_kind::CharacterKind::MerryManC
                    )
                )
            });
        let generic_only = self
            .mission_domain
            .achievements
            .record_party_composition(!party.is_empty(), generic_only);
        let optional = self
            .mission_domain
            .campaign
            .current_mission_idx
            .is_some_and(|idx| {
                let profile =
                    self.mission_domain.campaign.missions[idx].profile(&assets.profile_manager);
                matches!(
                    profile.mission_type,
                    crate::profiles::MissionType::Ambush | crate::profiles::MissionType::Tactical
                ) && profile.required_character_indices.iter().all(|&index| {
                    let character = &assets.profile_manager.characters[index as usize];
                    matches!(
                        crate::character_kind::CharacterKind::from_profile(
                            &character.filename,
                            &character.profile_name
                        ),
                        Some(
                            crate::character_kind::CharacterKind::MerryManA
                                | crate::character_kind::CharacterKind::MerryManB
                                | crate::character_kind::CharacterKind::MerryManC
                        )
                    )
                })
            });
        if self
            .mission_domain
            .achievements
            .verifiable_achievements()
            .contains(crate::achievement::AchievementId::PeopleBehindTheLegend)
        {
            use crate::achievement::{AchievementEvaluation as E, AchievementId as A};
            self.mission_domain
                .achievements
                .record_evaluation(
                    A::PeopleBehindTheLegend,
                    if !optional {
                        E::NotApplicable
                    } else if generic_only {
                        E::Earned
                    } else {
                        E::Failed
                    },
                )
                .expect("party achievement after finalization");
        }
        let npcs = self
            .world
            .entities
            .occupied()
            .filter_map(|(id, entity)| {
                let (camp, out_of_order) = match entity {
                    Entity::Soldier(soldier) => (soldier.camp(), soldier.is_out_of_order()),
                    Entity::Civilian(civilian) => (civilian.camp(), civilian.is_out_of_order()),
                    _ => return None,
                };
                let building = self
                    .entity_building_sector(entity.element_data().sector())
                    .map(|sector| AchievementBuildingId {
                        public_number: sector.get(),
                        arena_index: sector.arena_index(),
                    });
                Some(AchievementEntitySnapshot {
                    entity: id,
                    hostile: matches!(entity, Entity::Soldier(_)) && self.is_hostile_to_player_camp(camp),
                    dead: entity.is_dead(),
                    health: match entity { Entity::Soldier(s) => s.life_points() as i32, Entity::Civilian(c) => c.life_points() as i32, _ => unreachable!() },
                    bound: entity.element_data().posture() == crate::element::Posture::Tied,
                    unconscious: entity.human_data().expect("NPC has no human state").unconscious,
                    rich_civilian: match entity { Entity::Civilian(c) => {
                        let profile = assets.profile_manager.civilians.get(c.civilian.civilian_profile_index.0 as usize).expect("civilian has no profile");
                        matches!(profile.filename.as_str(), "ManCivilianRich" | "WomanCivilianRich")
                    }, _ => false },
                    beggar: matches!(entity, Entity::Civilian(c) if c.civilian.beggar_scroll_sets.as_ref().is_some_and(|sets| sets.iter().any(|set| !set.is_empty()))),
                    out_of_order,
                    building,
                })
            })
            .collect::<Vec<_>>();
        self.mission_domain
            .achievements
            .refresh_hostile_arrangement(self.control.frame_counter, npcs)
            .expect("achievement arrangement changed after mission finalization");
    }

    /// Classify a fresh death from the damage element's authoritative origin.
    pub(super) fn classify_achievement_death_cause(
        &self,
        origin: Option<EntityId>,
    ) -> AchievementDeathCause {
        let Some(origin) = origin else {
            return AchievementDeathCause::EnvironmentOrScript;
        };
        let entity = self.world.entities.get(origin).unwrap_or_else(|| {
            panic!(
                "damage origin {} disappeared before achievement responsibility was recorded",
                origin.index()
            )
        });
        match entity {
            Entity::Pc(_) => AchievementDeathCause::PlayerControlled,
            Entity::Soldier(_) => {
                let directly_controlled = self.players.tactical.orders.contains_key(&origin)
                    || self.tactical_unit_is_selected(origin);
                if directly_controlled {
                    AchievementDeathCause::PlayerControlled
                } else {
                    AchievementDeathCause::Npc
                }
            }
            Entity::Civilian(_) => AchievementDeathCause::Npc,
            Entity::Fx(_)
            | Entity::Target(_)
            | Entity::Bonus(_)
            | Entity::Scroll(_)
            | Entity::Projectile(_)
            | Entity::Net(_) => AchievementDeathCause::EnvironmentOrScript,
        }
    }

    pub(super) fn record_achievement_npc_death(
        &mut self,
        victim: EntityId,
        origin: Option<EntityId>,
    ) {
        let npc = self
            .world
            .entities
            .get(victim)
            .is_some_and(|entity| matches!(entity, Entity::Soldier(_) | Entity::Civilian(_)));
        if !npc || !self.mission_domain.achievements.is_fresh_death(victim) {
            return;
        }
        let cause = self.classify_achievement_death_cause(origin);
        if let Some(actor) = origin {
            self.record_achievement_contribution(actor);
            self.record_achievement_tactical_effect(actor, victim);
        }
        if matches!(self.world.entities.get(victim), Some(Entity::Civilian(_))) {
            self.mission_domain
                .achievements
                .latch(crate::achievement::AchievementId::KillCivilian);
        }
        self.mission_domain
            .achievements
            .record_npc_death(
                victim,
                cause,
                self.control.sim_config.clean_hands_npc_kills_invalidate,
            )
            .expect("NPC death arrived after achievement finalization");
    }

    /// Record the exact positive optical sample produced by Enemy detection.
    pub(super) fn record_achievement_hostile_observation(
        &mut self,
        observer: EntityId,
        pc: EntityId,
    ) {
        self.mission_domain
            .achievements
            .record_hostile_observation(observer, pc)
            .expect("hostile observation arrived after achievement finalization");
    }

    pub fn achievement_progress(&self) -> AchievementProgressSnapshot {
        self.mission_domain
            .achievements
            .progress(self.control.frame_counter)
    }

    /// Apply host eligibility once, after the deterministic terminal command
    /// has appended the raw attempt. Campaign attempts receive an immutable
    /// run-and-sequence-keyed attestation even when policy blocks awards.
    /// Custom/headless tools without a campaign attempt still expose the
    /// calculated decision, but have no canonical history record to mutate.
    pub(crate) fn promote_mission_achievement_results(
        &mut self,
        policy: crate::achievement::AchievementUnlockPolicy,
        mut context: crate::achievement::AchievementRunContext,
        profiles: &crate::profiles::ProfileManager,
    ) -> Result<Option<crate::achievement::AchievementHistoryUpdate>, String> {
        if self
            .mission_domain
            .achievements
            .history_promotion_attempted()
        {
            return Ok(None);
        }
        let results = self
            .mission_domain
            .achievements
            .finalized_results()
            .copied()
            .ok_or_else(|| {
                "achievement history promotion requires successful finalized results".to_string()
            })?;
        context.cheat_used |= self.mission_domain.cheat_used_flags != 0
            || !self.control.sim_config.script_enabled
            || self.control.sim_config.highlander
            || self.control.sim_config.highlander2
            || self.control.sim_config.golden_eye
            || self.control.sim_config.ignore_default_loose;
        let decision = policy.evaluate(context, results);
        if context.kind == crate::achievement::AchievementRunKind::CustomMission {
            self.mission_domain
                .achievements
                .mark_history_promotion_attempted();
            return Ok(Some(crate::achievement::AchievementHistoryUpdate {
                blockers: decision.blockers,
                newly_earned: crate::achievement::AchievementSet::empty(),
                mission_badges: crate::achievement::AchievementSet::empty(),
            }));
        }

        let Some(key) = self.mission_domain.campaign.latest_mission_attempt_key() else {
            if context.headless && !decision.may_persist() {
                self.mission_domain
                    .achievements
                    .mark_history_promotion_attempted();
                return Ok(Some(crate::achievement::AchievementHistoryUpdate {
                    blockers: decision.blockers,
                    newly_earned: crate::achievement::AchievementSet::empty(),
                    mission_badges: crate::achievement::AchievementSet::empty(),
                }));
            }
            return Err(
                "campaign achievement attestation requires the exact attempt appended by ApplyQuitMissionUpdates"
                    .to_string(),
            );
        };
        let update = self
            .mission_domain
            .campaign
            .attest_mission_achievement_attempt(key, policy, context, profiles)
            .map_err(|error| error.to_string())?;
        self.mission_domain
            .achievements
            .mark_history_promotion_attempted();
        Ok(Some(update))
    }

    /// Read exact campaign-owned XP; a live PC without its required campaign
    /// description is corruption, not a zero-XP character.
    pub fn pc_experience_snapshot(&self, entity: EntityId) -> Result<PcExperienceSnapshot, String> {
        let pc = self
            .world
            .entities
            .get(entity)
            .and_then(Entity::pc_data)
            .ok_or_else(|| format!("entity {} is not a player character", entity.index()))?;
        let description = self.pc_description_for_pc_data(pc).ok_or_else(|| {
            format!(
                "player character {} has no exact campaign description",
                entity.index()
            )
        })?;
        Ok(PcExperienceSnapshot {
            entity,
            hand_to_hand: description.status.human_status.hand_to_hand,
            bow: description.status.human_status.bow,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::achievement::{
        AchievementRunContext, AchievementRunKind, AchievementUnlockBlockers,
        AchievementUnlockPolicy,
    };
    use crate::diplomacy::{DiplomacyDefinition, DiplomacyState};
    use crate::element::{ActorSoldier, Camp, ElementData, ElementKind, NpcData, SoldierData};

    fn test_soldier(camp: Camp) -> Entity {
        Entity::Soldier(ActorSoldier {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorSoldier;
                initial_element.active = true;
                initial_element
            },
            actor: Default::default(),
            human: Default::default(),
            npc: NpcData {
                life_points: 100,
                ..Default::default()
            },
            soldier: SoldierData {
                cached_camp: camp,
                ..Default::default()
            },
        })
    }

    fn engine_with_custom_player_coalition() -> EngineInner {
        let mut engine = EngineInner::new();
        engine.mission_domain.diplomacy = DiplomacyState::from_definition(
            true,
            true,
            Some(&DiplomacyDefinition {
                player_coalition: vec![4],
                relationships: vec![],
            }),
        )
        .expect("custom player coalition should be valid");
        engine
    }

    fn finalized_engine_without_campaign_mission() -> EngineInner {
        let mut engine = EngineInner::new();
        engine
            .mission_domain
            .achievements
            .initialize_mission_baseline(0, []);
        engine.mission_domain.achievements.finalize_success();
        engine
    }

    #[test]
    fn custom_run_is_blocked_before_campaign_mission_lookup() {
        let mut engine = finalized_engine_without_campaign_mission();
        let update = engine
            .promote_mission_achievement_results(
                AchievementUnlockPolicy::default(),
                AchievementRunContext {
                    kind: AchievementRunKind::CustomMission,
                    ..AchievementRunContext::default()
                },
                &crate::profiles::ProfileManager::new(),
            )
            .expect("custom run should be calculated without campaign history")
            .expect("first promotion attempt returns a policy result");

        assert!(
            update
                .blockers
                .contains(AchievementUnlockBlockers::CUSTOM_MISSION)
        );
        assert!(update.newly_earned.is_empty());
    }

    #[test]
    fn gameplay_cheat_modes_block_unlocks_without_host_inference() {
        let mut engine = finalized_engine_without_campaign_mission();
        engine.control.sim_config.golden_eye = true;
        let update = engine
            .promote_mission_achievement_results(
                AchievementUnlockPolicy::default(),
                AchievementRunContext {
                    headless: true,
                    ..AchievementRunContext::default()
                },
                &crate::profiles::ProfileManager::new(),
            )
            .expect("cheated run should still produce its policy result")
            .expect("first promotion attempt returns a policy result");

        assert!(
            update
                .blockers
                .contains(AchievementUnlockBlockers::CHEAT_USED)
        );
        assert!(
            update
                .blockers
                .contains(AchievementUnlockBlockers::HEADLESS)
        );
        assert!(update.newly_earned.is_empty());
    }

    #[test]
    fn arrangement_uses_live_player_coalition_instead_of_royalist_fallback() {
        for (camp, expected_hostiles) in [(Camp::Royalists, 1), (Camp::Custom(4), 0)] {
            let mut engine = engine_with_custom_player_coalition();
            engine.add_entity(test_soldier(camp));

            engine.initialize_achievement_tracking(&super::super::LevelAssets::default());

            assert_eq!(
                engine.achievement_progress().metrics.encountered_hostiles,
                expected_hostiles,
                "unexpected hostile classification for {camp:?}"
            );
        }
    }

    #[test]
    fn selected_non_royalist_player_soldier_is_direct_player_causality() {
        let mut engine = engine_with_custom_player_coalition();
        let soldier = engine.add_entity(test_soldier(Camp::Custom(4)));
        engine.players.tactical.seats[0].selection.push(soldier);

        assert_eq!(
            engine.classify_achievement_death_cause(Some(soldier)),
            AchievementDeathCause::PlayerControlled
        );
    }
}
