//! Checkpoint reunions and officer referral execute against live brains.
use super::*;
use crate::ai::{
    AiEntityHandle, AiState, AlertLevel, DutyFlags, Remark, SpeechFlags, Stimulus, StimulusInfo,
    StoredEnumWord, Substate,
};
use crate::ai_enemy::SeekFlags;
use crate::profiles::ProfileRank;
#[cfg(test)]
use crate::sim_rng::SimulationContext;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::test_support::actors::make_test_ai_soldier;

    #[test]
    fn sync_reunion_waits_for_enroute_partners_last_waypoint() {
        let mut engine = EngineInner::new();
        let owner =
            engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
        let partner =
            engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.control.frame_counter = 1072;
        let ai = &mut engine.observation_ai_mut(owner).base;
        ai.current_state = AiState::Default;
        ai.current_substate = Substate::DefaultLookingSidewardsForCharly;
        ai.synchronize_charly = Some(AiEntityHandle::new(partner.index()));
        ai.synchronize_index = 3;
        ai.macro_in_progress = true;
        ai.macro_command = vec![crate::ai::MacroOpcode::Wait as u8, 100, 0];
        ai.macro_command_offset = 0;
        ai.number_of_remaining_macro_bytes = 3;
        let ai = &mut engine.observation_ai_mut(partner).base;
        ai.current_state = AiState::Default;
        ai.current_substate = Substate::DefaultEnroute;
        ai.macro_in_progress = false;
        ai.detached_patrol_path_status.current_waypoint_index = 3;
        ai.detached_patrol_path_status.last_waypoint_index = 2;
        engine.execute_ai_seen_charly(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            partner.index(),
        );
        let ai = &engine.observation_ai(owner).base;
        assert_eq!(ai.current_substate, Substate::DefaultSynchronizing);
        assert_eq!(ai.macro_command_offset, 0);
        assert_eq!(ai.number_of_remaining_macro_bytes, 3);
        assert!(!ai.macro_timer_is_running);
        assert_eq!(ai.when_does_timer_ring, 1092);
        assert_eq!(
            engine.observation_ai(partner).base.synchronizing_actors,
            vec![owner.index()]
        );
    }
}

impl EngineInner {
    #[cfg(test)]
    pub(in crate::engine) fn execute_ai_seen_charly(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: u32,
    ) {
        AiOwnerCtx::new(self, sim, assets, owner).execute_ai_seen_charly(target)
    }

    #[cfg(test)]
    pub(in crate::engine) fn unalert_live_charly_seekers(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        charly: EntityId,
    ) {
        AiOwnerCtx::new(self, sim, assets, owner).unalert_live_charly_seekers(charly)
    }
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_ai_seen_charly(&mut self, target: u32) {
        let charly = self
            .engine
            .expect_human_id_for_ai_handle(target, "seen checkpoint");
        if self.engine.observation_ai(self.owner).base.current_state == AiState::Seeking {
            match self
                .engine
                .observation_ai(self.owner)
                .get_rank(&self.assets.profile_manager)
            {
                ProfileRank::Officer => {
                    if matches!(
                        self.engine.observation_ai(self.owner).base.current_substate,
                        Substate::SeekingOfficerWaitForCharly
                            | Substate::SeekingOfficerLectureCharly
                    ) {
                        return;
                    }
                    if self
                        .engine
                        .enemy_ai(charly, "checkpoint report status")
                        .reported_to_officer
                    {
                        return;
                    }
                    self.unalert_live_charly_seekers(charly);
                    if self
                        .engine
                        .expect_entity(charly, "checkpoint rank")
                        .enemy_ai()
                        .is_some_and(|ai| {
                            ai.get_rank(&self.assets.profile_manager) == ProfileRank::Soldier
                        })
                    {
                        self.observation_say(Remark::FoundCharly);
                        let mut call = Stimulus::new(StimulusType::CallGoToOfficer);
                        call.info = StimulusInfo::Human(AiEntityHandle::new(self.owner.index()));
                        self.engine
                            .execute_ai_callback(self.sim, self.assets, charly, &call);
                        self.engine.observation_ai_mut(self.owner).base.antagonist =
                            Some(AiEntityHandle::new(target));
                        assert_eq!(
                            self.engine
                                .world
                                .entities
                                .expect_enemy_ai(charly, format_args!("called checkpoint rank"))
                                .get_rank(&self.assets.profile_manager),
                            ProfileRank::Soldier
                        );
                        self.observation_face_entity(charly, false);
                        self.duty_set_state(
                            AiState::Seeking,
                            Substate::SeekingOfficerWaitForCharly,
                        );
                        self.engine.observation_timer(self.owner, 10);
                        return;
                    }
                }
                ProfileRank::Soldier => {
                    if self
                        .engine
                        .observation_ai(self.owner)
                        .base
                        .antagonist
                        .is_some()
                        && self
                            .engine
                            .expect_entity(charly, "checkpoint referral")
                            .enemy_ai()
                            .is_some_and(|ai| {
                                ai.get_rank(&self.assets.profile_manager) == ProfileRank::Soldier
                                    && !ai.reported_to_officer
                            })
                    {
                        self.engine
                            .observation_ai_mut(self.owner)
                            .seek_flags
                            .remove(SeekFlags::REPORT_OFFICER_AFTER);
                        let state = self
                            .engine
                            .ai(charly, "checkpoint referral state")
                            .current_substate;
                        if matches!(
                            state,
                            Substate::SeekingCharlySentToOfficer
                                | Substate::SeekingCharlyGoToOfficer
                                | Substate::SeekingCharlyGoToOfficerSeen
                                | Substate::SeekingCharlyGetLectureByOfficer
                                | Substate::SeekingCharlyGetLectureByOfficer2
                        ) {
                            self.execute_ai_return_to_duty(DutyFlags::empty());
                            return;
                        }
                        self.duty_set_state(AiState::Seeking, Substate::SeekingSendCharlyToOfficer);
                        self.unalert_live_charly_seekers(charly);
                        self.execute_ai_speech(crate::ai::AiSpeechAttempt {
                            remark: Remark::FoundCharly,
                            flags: SpeechFlags::MYTALK_1.bits(),
                        });
                        self.engine
                            .observation_ai_mut(self.owner)
                            .base
                            .friend_in_trouble = Some(AiEntityHandle::new(target));
                        self.observation_face_entity(charly, false);
                        return;
                    }
                }
                ProfileRank::Knight | ProfileRank::None => {}
            }
            self.observation_say(Remark::FoundCharly);
        }
        self.engine.observation_ai_mut(self.owner).base.sorrow_level = 0;
        self.engine
            .execute_ai_set_checkpoint_charly(self.owner, None);

        let ai = &self.engine.observation_ai(self.owner).base;
        if ai.synchronize_index == u16::MAX
            || ai.synchronize_charly.is_none()
            || !ai.macro_in_progress
        {
            self.engine.halt_actor(self.sim, self.assets, self.owner);

            self.engine.execute_ai_set_alert_status(
                self.assets,
                self.owner,
                AlertLevel::Green,
                crate::ai::AlertFlags::empty(),
            );

            self.observation_face_entity(charly, false);
            if self.engine.observation_ai(self.owner).base.current_state == AiState::Default {
                self.duty_set_state(AiState::Default, Substate::DefaultDetectedCharly);
            } else {
                let ai = self.engine.observation_ai_mut(self.owner);
                ai.previous_state = StoredEnumWord::new(ai.base.current_state);
                ai.previous_substate = StoredEnumWord::new(ai.base.current_substate);
                self.unalert_live_charly_seekers(charly);
                self.duty_set_state(AiState::Seeking, Substate::SeekingDetectedCharly);
            }
            self.engine
                .observation_timer(self.owner, crate::parameters_ai::AI_CHARLY_LOOK_TIME as u32);
            return;
        }
        let friend = self.engine.expect_human_id_for_ai_handle(
            ai.synchronize_charly.unwrap().get(),
            "checkpoint synchronization partner",
        );
        let partner = self.engine.ai(friend, "checkpoint synchronization partner");
        let index = ai.synchronize_index;
        let partner_default = partner.current_state == AiState::Default;
        let path = partner.patrol_path.as_ref();
        let there = if partner.macro_in_progress {
            path.map_or(
                partner.detached_patrol_path_status.current_waypoint_index,
                |path| path.current_waypoint_index,
            ) as u16
                == index
        } else if partner.current_substate == Substate::DefaultEnroute {
            path.map_or(
                partner.detached_patrol_path_status.last_waypoint_index,
                |path| path.last_waypoint_index,
            ) as u16
                == index
        } else {
            false
        };
        if !partner_default || there {
            self.duty_set_state(AiState::Default, Substate::DefaultInMacro);
            self.run_ai_macro();
        } else {
            self.engine
                .ai_mut(friend, "checkpoint synchronization registration")
                .synchronizing_actors
                .push(self.owner.index());
            self.duty_set_state(AiState::Default, Substate::DefaultSynchronizing);
            self.engine.observation_timer(self.owner, 20);
        }
    }

    pub(in crate::engine) fn unalert_live_charly_seekers(&mut self, charly: EntityId) {
        let count = self.engine.entities().len();
        for index in 0..count {
            let Some((candidate, Entity::Soldier(_))) =
                self.engine.entities().get_legacy_slot(index as u32)
            else {
                continue;
            };
            if candidate == self.owner || candidate == charly {
                continue;
            }
            let ai = self.engine.observation_ai(self.owner);
            if ai.get_rank(&self.assets.profile_manager) != ProfileRank::Officer
                && ai
                    .base
                    .antagonist
                    .is_some_and(|target| target.get() == candidate.index())
            {
                continue;
            }
            if self
                .engine
                .live_ai_detects_180(self.assets, candidate, charly)
                || charly != self.owner
                    && self
                        .engine
                        .live_ai_detects_180(self.assets, candidate, self.owner)
            {
                self.engine.execute_ai_callback(
                    self.sim,
                    self.assets,
                    candidate,
                    &Stimulus::with_human(StimulusType::CallCharlyIsBack, charly.index()),
                );
            }
        }
    }
}
