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
    /// Capture the baseline after startup scripts and Sherwood production
    /// setup have settled. This is deliberately later than level parsing:
    /// scripted setup deaths must not be attributed to the player.
    pub(super) fn initialize_achievement_tracking(&mut self) {
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
        self.refresh_achievement_progress();
    }

    /// Refresh all arrangement-derived progress from exact live entity and
    /// sector state. The scan order is irrelevant; the tracker uses ordered
    /// identity sets/maps and is included in replay/rollback state.
    pub(super) fn refresh_achievement_progress(&mut self) {
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
                    hostile: camp.is_hostile_to(crate::element::Camp::Royalists),
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
        if !npc {
            return;
        }
        let cause = self.classify_achievement_death_cause(origin);
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
    use crate::element::{ActorSoldier, Camp, ElementData, ElementKind, NpcData, SoldierData};

    fn test_soldier(camp: Camp) -> Entity {
        Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                active: true,
                ..Default::default()
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
    fn arrangement_uses_original_player_camp_hostility() {
        let mut engine = EngineInner::new();
        engine.add_entity(test_soldier(Camp::Lacklandists));
        engine.add_entity(test_soldier(Camp::Royalists));

        engine.initialize_achievement_tracking();

        assert_eq!(
            engine.achievement_progress().metrics.encountered_hostiles,
            1
        );
    }

    #[test]
    fn selected_player_soldier_is_direct_player_causality() {
        let mut engine = EngineInner::new();
        let soldier = engine.add_entity(test_soldier(Camp::Royalists));
        engine.players.tactical.seats[0].selection.push(soldier);

        assert_eq!(
            engine.classify_achievement_death_cause(Some(soldier)),
            AchievementDeathCause::PlayerControlled
        );
    }
}
