use super::*;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_dispatch_patrol_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        mut stimulus: crate::ai::Stimulus,
    ) {
        if !self.dispatch_live_stimulus_to_patrol(sim, assets, owner, &stimulus) {
            stimulus.to_whole_patrol = true;
            self.resume_local_patrol_stimulus(sim, assets, owner, &stimulus);
        }
    }

    fn resume_local_patrol_stimulus(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &crate::ai::Stimulus,
    ) {
        let target = match stimulus.info {
            crate::ai::StimulusInfo::Human(handle)
                if matches!(
                    stimulus.stimulus_type,
                    crate::ai::StimulusType::EventView
                        | crate::ai::StimulusType::EventOutOfView
                        | crate::ai::StimulusType::EventSeesBeggar
                        | crate::ai::StimulusType::EventEnemyNear
                ) =>
            {
                Some(self.expect_entity_id_for_index(handle.get(), "patrol detection target"))
            }
            _ => None,
        };
        self.execute_ai_handler_body(sim, assets, owner, stimulus, target);
    }

    pub(in crate::engine) fn dispatch_live_stimulus_to_patrol(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &crate::ai::Stimulus,
    ) -> bool {
        use crate::ai::{AiState, StimulusType, Substate};

        if stimulus.to_whole_patrol {
            return false;
        }
        let ai = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("patrol dispatch owner"));
        if matches!(
            stimulus.stimulus_type,
            StimulusType::EventSeesObject | StimulusType::EventHear | StimulusType::EventSeesBody
        ) && ai
            .last_stimulus_dispatched_to_patrol
            .as_ref()
            .is_some_and(|last| last.is_similar(stimulus))
        {
            return true;
        }
        match ai.base.current_state {
            AiState::Default
                if ai.base.current_substate != Substate::DefaultPatrolEnrouteRunning => {}
            AiState::Wondering => {}
            _ => return false,
        }
        if let Some(chief) = ai.base.patrol_chief {
            if matches!(
                self.world
                    .entities
                    .expect_entity(chief, format_args!("patrol chief")),
                Entity::Soldier(_)
            ) && self.patrol_member_visible(assets, owner, chief)
            {
                return self.dispatch_live_stimulus_to_patrol(sim, assets, chief, stimulus);
            }
        }

        let ai = self
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("patrol dispatch owner"));
        ai.last_stimulus_dispatched_to_patrol = Some(*stimulus);
        if ai.base.patrol.is_empty() {
            return false;
        }
        // This call intentionally retains membership before recursively
        // processing the chief, which can rebuild the live patrol list.
        let members = ai.base.patrol.iter().map(|member| member.index()).collect();
        let mut forwarded = *stimulus;
        forwarded.to_whole_patrol = true;
        self.execute_ai_patrol_broadcast(sim, assets, owner, forwarded, members);
        true
    }

    pub(in crate::engine) fn execute_ai_patrol_broadcast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        source_id: EntityId,
        stimulus: crate::ai::Stimulus,
        members: Vec<u32>,
    ) {
        self.execute_ai_callback(sim, assets, source_id, &stimulus);
        self.execute_ai_patrol_member_broadcast(sim, assets, source_id, &stimulus, members);
    }

    fn execute_ai_patrol_member_broadcast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        source_id: EntityId,
        stimulus: &crate::ai::Stimulus,
        members: Vec<u32>,
    ) {
        for member in members {
            let member_id = self.entity_id_for_index(member).unwrap_or_else(|| {
                panic!(
                    "patrol broadcast from chief {} references missing member {member}",
                    source_id.index()
                )
            });

            let detected = matches!(
                self.world
                    .entities
                    .expect_entity(member_id, format_args!("patrol broadcast member")),
                Entity::Soldier(_)
            ) && self.patrol_member_visible(assets, source_id, member_id);
            tracing::trace!(
                target: "patrol_relay",
                chief = source_id.index(),
                member,
                stimulus_type = ?stimulus.stimulus_type,
                detected,
                "patrol broadcast member gate"
            );
            if !detected {
                continue;
            }

            self.execute_ai_callback(sim, assets, member_id, stimulus);
        }
    }
}
