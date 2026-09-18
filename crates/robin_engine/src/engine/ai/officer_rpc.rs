//! Direct officer and checkpoint calls preserve their caller stack.
#[cfg(test)]
mod tests;
use super::*;
use crate::ai::{
    AiEntityHandle, AiState, EmoticonType, GotoFlags, Position, Remark, SpeechFlags, Stimulus,
    StimulusInfo, Substate,
};
use crate::ai_enemy::SeekFlags;
use crate::profiles::ProfileRank;
use crate::sim_rng::SimulationContext;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_officer_rpc(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        let event = stimulus.stimulus_type;
        let substate = self.observation_ai(owner).base.current_substate;
        if matches!(
            substate,
            Substate::DefaultLookingForCharly
                | Substate::DefaultLookingSidewardsForCharly
                | Substate::DefaultDetectedCharly
                | Substate::DefaultSynchronizing
        ) && matches!(
            event,
            StimulusType::EventTimer | StimulusType::EventDone | StimulusType::EventSyncCharly
        ) {
            AiOwnerCtx::new(self, sim, assets, owner).charly_event(substate, stimulus);
            return Some(false);
        }
        if matches!(
            event,
            StimulusType::EventTimer
                | StimulusType::EventReachPoint
                | StimulusType::EventDone
                | StimulusType::EventMyTalk1
                | StimulusType::EventMyTalk2
                | StimulusType::EventMyTalk3
                | StimulusType::CallYourTalk1
                | StimulusType::CallYourTalk2
                | StimulusType::CallCoordinate
        ) && matches!(
            substate,
            Substate::SeekingDetectedCharly
                | Substate::SeekingSendCharlyToOfficer
                | Substate::SeekingLookingResurrectedCharly
                | Substate::SeekingCharlySentToOfficer
                | Substate::SeekingCharlyGoToOfficerSeen
                | Substate::SeekingCharlyGetLectureByOfficer
                | Substate::SeekingCharlyGetLectureByOfficer2
                | Substate::SeekingOfficerWaitForCharly
                | Substate::SeekingOfficerLectureCharly
                | Substate::SeekingOfficerLectureCharlyPointing
                | Substate::SeekingCharly
                | Substate::SeekingCharlyWatching
        ) {
            AiOwnerCtx::new(self, sim, assets, owner).charly_event(substate, stimulus);
            return Some(false);
        }
        if self.observation_ai(owner).base.current_substate == Substate::SeekingCharlyGoToOfficer
            && matches!(
                event,
                StimulusType::EventTimer | StimulusType::EventReachPoint
            )
        {
            AiOwnerCtx::new(self, sim, assets, owner).report_back(event);
            return Some(false);
        }
        if !matches!(
            event,
            StimulusType::EventSeesSoldier
                | StimulusType::CallAlert
                | StimulusType::CallHey
                | StimulusType::CallGoToOfficer
                | StimulusType::CallMrOfficerIAmBack
                | StimulusType::CallCharlyIsBack
        ) {
            return None;
        }
        let StimulusInfo::Human(target) = stimulus.info else {
            panic!("officer call {event:?} requires a human sender");
        };
        let target = self.expect_human_id_for_ai_handle(target.get(), "officer call sender");
        Some(AiOwnerCtx::new(self, sim, assets, owner).rpc_event(event, target))
    }
}

impl AiOwnerCtx<'_> {
    fn halt(&mut self) {
        self.engine.halt_actor(self.sim, self.assets, self.owner);
    }
    fn face(&mut self, target: EntityId) {
        self.engine
            .observation_face_entity(self.sim, self.assets, self.owner, target, false);
    }
    fn face_position(&mut self, point: Position) {
        let target = self.engine.position_to_point_3d(
            self.assets,
            point.sector,
            point.level,
            point.x,
            point.y,
        );
        let body = self
            .engine
            .expect_entity(self.owner, "officer facing")
            .element_data()
            .position();
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
            target.x - body.x,
            target.y - body.y,
        );
        self.engine
            .duty_face_direction(self.sim, self.assets, self.owner, direction as u16);
    }
    fn emoticon(&mut self, emoticon: EmoticonType) {
        let frame = self.engine.control.frame_counter;
        self.enemy_mut()
            .base
            .set_transient_emoticon(emoticon, 20, frame);
    }
    fn priority(&self) -> bool {
        self.enemy().base.blood_alcohol as i32 <= crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            && self.enemy().has_the_new_task_priority()
    }
    fn rank(&self, target: EntityId) -> ProfileRank {
        self.engine
            .enemy_ai(target, "officer call rank")
            .get_rank(&self.assets.profile_manager)
    }
    fn accept(&mut self, state: Substate) {
        self.seek_state(state);
        self.timer(20);
        self.emoticon(EmoticonType::QuestionMark);
    }
    fn rpc_event(&mut self, event: StimulusType, target: EntityId) -> bool {
        use StimulusType::*;
        use Substate::*;
        match event {
            EventSeesSoldier => {
                if self.enemy().base.current_state != AiState::Default
                    && !(self.enemy().base.current_state == AiState::Seeking
                        && matches!(
                            self.enemy().base.current_substate,
                            SeekingOfficerLookingForSoldiers1
                                | SeekingOfficerLookingForSoldiers1Sidewards
                                | SeekingOfficerLookingForSoldiers2
                                | SeekingOfficerLookingForSoldiers2Sidewards
                                | SeekingOfficerLookingForSoldiers3
                                | SeekingOfficerLookingForSoldiers3Sidewards
                                | SeekingRunningToOfficer
                        ))
                {
                    return false;
                }
                match self.enemy().get_rank(&self.assets.profile_manager) {
                    ProfileRank::Soldier => {
                        self.enemy_mut().base.antagonist =
                            Some(AiEntityHandle::new(target.index()));
                        if self.engine.execute_ai_callback(
                            self.sim,
                            self.assets,
                            target,
                            &Stimulus::with_human(CallAlert, self.owner.index()),
                        ) {
                            self.seek_state(SeekingRunningToOfficerSeen);
                            self.say(Remark::CallsOfficer, SpeechFlags::MYTALK_0);
                            let officer = self.target();
                            let input = extract_exact_forecast_input(
                                self.engine,
                                self.engine.expect_entity(officer, "officer forecast"),
                                selected_actor_is_passing_door(
                                    &self.engine.entities(),
                                    &self.engine.seq(),
                                    officer,
                                ),
                            )
                            .expect("officer forecast requires actor");
                            let position = crate::ai::forecast_destination_for_ia(
                                self.sim,
                                &input,
                                &self.engine.script_domains.interactables.doors,
                                &self.engine.world.fast_grid.level.sectors,
                                &self.engine.world.fast_grid.level.sector_number_map,
                            )
                            .position;
                            self.engine.duty_go_near(
                                self.sim,
                                self.assets,
                                self.owner,
                                position,
                                crate::parameters_ai::AI_TALK_DISTANCE,
                                GotoFlags::RUN,
                            );
                            self.timer(20);
                        } else {
                            self.duty();
                        }
                    }
                    ProfileRank::Officer => {
                        assert!(matches!(
                            self.engine.expect_entity(target, "seen soldier"),
                            Entity::Soldier(_)
                        ));
                        self.enemy_mut().base.antagonist =
                            Some(AiEntityHandle::new(target.index()));
                        if self.engine.can_call_ai_soldier(self.owner, target) {
                            self.face(self.target());
                            self.seek_state(SeekingOfficerCallSoldier);
                            self.engine.execute_ai_delete_detectable_type(
                                self.owner,
                                crate::element::DetectableType::Friend,
                            );
                        }
                    }
                    ProfileRank::Knight | ProfileRank::None => {}
                }
            }
            CallGoToOfficer => {
                if self.enemy().base.current_state != AiState::Default
                    && self.enemy().base.current_substate != SleepingAwakening
                {
                    return false;
                }
                self.enemy_mut().base.antagonist = Some(AiEntityHandle::new(target.index()));
                self.seek_state(SeekingCharlySentToOfficer);
                self.enemy_mut().base.set_emoticon(EmoticonType::None);

                self.timer(30);
                self.enemy_mut().reported_to_officer = true;
                return true;
            }
            CallHey => {
                self.enemy_mut().base.antagonist = Some(AiEntityHandle::new(target.index()));
                if matches!(
                    self.engine.expect_entity(target, "officer hail"),
                    Entity::Civilian(_)
                ) {
                    tracing::warn!("civilian officer hail is unsupported");
                    return false;
                }
                let react = matches!(
                    self.enemy().base.current_state,
                    AiState::Default | AiState::Wondering
                ) || self.enemy().base.current_state == AiState::Seeking
                    && matches!(
                        self.enemy().base.current_substate,
                        SeekingRunningToOfficer
                            | SeekingRunningToOfficerSeen
                            | SeekingHeardstepsReactiontime
                            | SeekingBodyReactiontime
                    );
                if !react
                    || self.enemy().get_rank(&self.assets.profile_manager) != ProfileRank::Soldier
                    || !self.priority()
                {
                    return false;
                }
                self.enemy_mut().current_task_priority = self.enemy().new_task_priority;
                self.engine
                    .observation_stop(self.sim, self.assets, self.owner);
                assert_eq!(self.rank(self.target()), ProfileRank::Officer);
                self.face(self.target());
                self.accept(SeekingSoldierCalledByOfficer);
                return true;
            }
            CallAlert => {
                self.enemy_mut().base.antagonist = Some(AiEntityHandle::new(target.index()));
                if matches!(
                    self.engine.expect_entity(target, "alert sender"),
                    Entity::Civilian(_)
                ) {
                    if self.enemy().base.current_state != AiState::Default {
                        return false;
                    }
                    self.halt();
                    self.face(self.target());
                    self.accept(SeekingWaitForAlertingCivilian);
                    return true;
                }
                match self.enemy().get_rank(&self.assets.profile_manager) {
                    ProfileRank::Soldier => {
                        let react = matches!(
                            self.enemy().base.current_state,
                            AiState::Default | AiState::Wondering
                        ) || self.enemy().base.current_state == AiState::Seeking
                            && matches!(
                                self.enemy().base.current_substate,
                                SeekingSoldierGiveReportToOfficer
                                    | SeekingSoldierGiveAlertingReportToOfficerStart
                                    | SeekingSoldierGiveAlertingReportToOfficerPoint
                                    | SeekingSoldierGiveAlertingReportToOfficerEnd
                            );
                        if !react || !self.priority() {
                            return false;
                        }
                        self.halt();
                        self.enemy_mut().current_task_priority = self.enemy().new_task_priority;
                        self.enemy_mut().gather_position_instructed = false;
                        self.enemy_mut().base.friends_are_alerted = true;
                        assert_eq!(self.rank(self.target()), ProfileRank::Officer);
                        let position = self.engine.live_ai_position(self.target());
                        self.enemy_mut().officers_position = position;
                        self.face_position(self.enemy().officers_position);
                        self.accept(SeekingGroupCalledByOfficer);
                        return true;
                    }
                    ProfileRank::Officer => {
                        if !self.officer_ready() {
                            return false;
                        }
                        self.halt();
                        self.enemy_mut().base.friends_are_alerted = true;
                        assert_eq!(self.rank(self.target()), ProfileRank::Soldier);
                        self.face(self.target());
                        self.accept(SeekingOfficerWaitForAlertingSoldier);
                        return true;
                    }
                    ProfileRank::Knight | ProfileRank::None => {
                        tracing::warn!("unsupported alert recipient rank");
                    }
                }
            }
            CallMrOfficerIAmBack => {
                self.enemy_mut().base.antagonist = Some(AiEntityHandle::new(target.index()));
                if self.enemy().base.current_state == AiState::Seeking
                    && self.enemy().base.current_substate == SeekingOfficerWaitForCharly
                {
                    return true;
                }
                if !self.officer_ready() {
                    return false;
                }
                self.halt();
                assert_eq!(self.rank(self.target()), ProfileRank::Soldier);
                self.face(self.target());
                self.seek_state(SeekingOfficerWaitForCharly);
                self.say(Remark::FoundCharly, SpeechFlags::empty());
                self.timer(20);
                self.emoticon(EmoticonType::XMark);
                return true;
            }
            CallCharlyIsBack => {
                let s = self.enemy().base.current_substate;
                if s.is_seek_area()
                    || matches!(
                        s,
                        SeekingSoldierReturnToOfficer
                            | SeekingSoldierGiveReportToOfficer
                            | SeekingBodyReactiontime
                            | SeekingBody
                            | SeekingNet
                            | SeekingGroupGetInstructedByOfficer
                    )
                {
                    if self.enemy().base.my_reconnaissance_report.charly
                        == Some(AiEntityHandle::new(target.index()))
                    {
                        self.engine
                            .execute_ai_set_checkpoint_charly(self.owner, Option::None);

                        self.face(target);
                        self.enemy_mut().base.clear_emoticon();

                        self.enemy_mut()
                            .seek_flags
                            .remove(SeekFlags::REPORT_OFFICER_AFTER);
                        self.seek_state(SeekingLookingResurrectedCharly);
                        let human = self.engine.expect_entity(target, "returned checkpoint");
                        self.timer(if human.is_dead() || human.is_unconscious() {
                            200
                        } else {
                            20
                        });
                    }
                } else {
                    self.engine
                        .execute_ai_set_checkpoint_charly(self.owner, Option::None);
                }
            }
            _ => unreachable!(),
        }
        false
    }
    fn officer_ready(&self) -> bool {
        self.enemy().base.current_state == AiState::Default
            || self.enemy().base.current_state == AiState::Seeking
                && matches!(
                    self.enemy().base.current_substate,
                    Substate::SeekingOfficerWaitForInstructedGroup
                        | Substate::SeekingOfficerWaitForInstructedSoldier
                )
    }
    fn charly_event(&mut self, state: Substate, stimulus: &Stimulus) {
        use StimulusType::*;
        use Substate::*;
        let event = stimulus.stimulus_type;
        match (state, event) {
            (DefaultLookingForCharly, EventTimer) => {
                let draw =
                    crate::sim_rng::u32(self.sim, crate::sim_rng::RngSite::CharlySorrow, 0..5000);
                if draw < u32::from(self.enemy().base.sorrow_level) + 10 {
                    self.engine.duty_set_state(
                        self.sim,
                        self.assets,
                        self.owner,
                        AiState::Default,
                        DefaultLookingSidewardsForCharly,
                    );
                    let direction = if crate::sim_rng::u32(
                        self.sim,
                        crate::sim_rng::RngSite::CharlySorrow,
                        0..2,
                    ) != 0
                    {
                        crate::ai::LookDirection::LeftRight
                    } else {
                        crate::ai::LookDirection::RightLeft
                    };
                    self.engine.execute_ai_look_sidewards(
                        self.sim,
                        self.assets,
                        self.owner,
                        direction,
                    );
                }
                self.enemy_mut().base.sorrow_level = self
                    .enemy()
                    .base
                    .sorrow_level
                    .wrapping_add(self.enemy().base.delta_sorrow_level);
                if self.enemy().base.sorrow_level > 1000 {
                    self.enemy_mut().base.sorrow_level = 0;
                    self.engine
                        .execute_ai_search_charly(self.sim, self.assets, self.owner);
                }
                self.timer(crate::parameters_ai::AI_CHECKFOR_TIME_INTERVAL as u32);
            }
            (DefaultLookingSidewardsForCharly, EventDone) => {
                self.engine.duty_set_state(
                    self.sim,
                    self.assets,
                    self.owner,
                    AiState::Default,
                    DefaultLookingForCharly,
                );
                self.timer(10);
            }
            (DefaultDetectedCharly, EventTimer) => {
                if self.enemy().base.macro_in_progress {
                    self.engine.duty_set_state(
                        self.sim,
                        self.assets,
                        self.owner,
                        AiState::Default,
                        DefaultInMacro,
                    );
                    self.engine.run_ai_macro(self.sim, self.assets, self.owner);
                } else {
                    self.duty();
                }
            }
            (DefaultSynchronizing, EventTimer) => {
                let partner = self
                    .enemy()
                    .base
                    .synchronize_charly
                    .expect("synchronization timer requires partner");
                let partner = self
                    .engine
                    .expect_human_id_for_ai_handle(partner.get(), "synchronization partner");
                if self
                    .engine
                    .ai(partner, "synchronization partner")
                    .current_state
                    != AiState::Default
                    || self
                        .engine
                        .expect_entity(partner, "synchronization partner")
                        .is_dead()
                {
                    self.duty();
                } else {
                    self.timer(20);
                }
            }
            (DefaultSynchronizing, EventSyncCharly) => {
                if matches!(stimulus.info, StimulusInfo::Index(index) if index == self.enemy().base.synchronize_index)
                {
                    assert!(
                        self.enemy().base.macro_in_progress,
                        "synchronization resumes active macro"
                    );
                    self.engine.duty_set_state(
                        self.sim,
                        self.assets,
                        self.owner,
                        AiState::Default,
                        DefaultInMacro,
                    );
                    self.engine.run_ai_macro(self.sim, self.assets, self.owner);
                }
            }
            (SeekingDetectedCharly, EventTimer) => {
                self.enemy_mut().base.my_reconnaissance_report.charly_seen = true;
                if self.enemy().get_rank(&self.assets.profile_manager) == ProfileRank::Officer
                    && !self.enemy().alerted_us.is_empty()
                {
                    let state = self.enemy().previous_state.get("previous state");
                    let substate = self.enemy().previous_substate.get("previous substate");
                    self.engine
                        .duty_set_state(self.sim, self.assets, self.owner, state, substate);
                    self.timer(10);
                } else {
                    self.duty();
                }
            }
            (SeekingSendCharlyToOfficer, EventMyTalk1) => {
                if let Some(friend) = self.enemy().base.friend_in_trouble {
                    let friend = self
                        .engine
                        .expect_human_id_for_ai_handle(friend.get(), "checkpoint referral");
                    assert!(
                        matches!(
                            self.engine.world.entities.get(friend),
                            Some(Entity::Soldier(_))
                        ),
                        "checkpoint referral requires enemy-soldier target"
                    );
                    let target = self.target();
                    if self.engine.execute_ai_callback(
                        self.sim,
                        self.assets,
                        friend,
                        &Stimulus::with_human(CallGoToOfficer, target.index()),
                    ) {
                        self.say(Remark::SendsCharlyToOfficer, SpeechFlags::MYTALK_2);
                        self.engine.duty_point_to(
                            self.sim,
                            self.assets,
                            self.owner,
                            self.enemy().officers_position,
                        );
                    }
                } else {
                    self.duty();
                }
            }
            (SeekingSendCharlyToOfficer, EventMyTalk2) => {
                self.seek_state(SeekingLookingResurrectedCharly);
                let friend = self
                    .enemy()
                    .base
                    .friend_in_trouble
                    .expect("checkpoint referral requires friend");
                let friend = self
                    .engine
                    .expect_human_id_for_ai_handle(friend.get(), "checkpoint referral face");
                self.face(friend);
                self.timer(100);
            }
            (SeekingLookingResurrectedCharly, EventTimer) => self.duty(),
            (SeekingCharlySentToOfficer, EventTimer) => {
                self.seek_state(SeekingCharlyGoToOfficer);
                let position = self.engine.live_ai_position(self.target());
                self.engine.duty_go_near(
                    self.sim,
                    self.assets,
                    self.owner,
                    position,
                    40,
                    GotoFlags::empty(),
                );
                self.engine.unalert_live_charly_seekers(
                    self.sim,
                    self.assets,
                    self.owner,
                    self.owner,
                );
                self.timer(10);
            }
            (SeekingCharlyGoToOfficerSeen, EventTimer) => {
                if self
                    .engine
                    .ai(self.target(), "checkpoint officer")
                    .current_substate
                    == SeekingOfficerWaitForCharly
                {
                    self.engine.unalert_live_charly_seekers(
                        self.sim,
                        self.assets,
                        self.owner,
                        self.owner,
                    );
                    self.timer(20);
                } else {
                    self.duty();
                }
            }
            (SeekingCharlyGoToOfficerSeen, EventReachPoint) => {
                self.engine.execute_ai_callback(
                    self.sim,
                    self.assets,
                    self.target(),
                    &Stimulus::with_human(CallCoordinate, self.owner.index()),
                );
                self.seek_state(SeekingCharlyGetLectureByOfficer);
            }
            (SeekingCharlyGetLectureByOfficer, CallYourTalk1) => {
                self.say(Remark::CharlyDefendsHimself, SpeechFlags::MYTALK_1);
                self.seek_state(SeekingCharlyGetLectureByOfficer2);
            }
            (SeekingCharlyGetLectureByOfficer2, EventMyTalk1)
            | (SeekingOfficerLectureCharly, EventMyTalk1) => {
                self.engine.execute_ai_callback(
                    self.sim,
                    self.assets,
                    self.target(),
                    &Stimulus::new(CallYourTalk1),
                );
            }
            (SeekingCharlyGetLectureByOfficer2, CallYourTalk2) => self.duty(),
            (SeekingOfficerWaitForCharly, EventTimer) => {
                if matches!(
                    self.engine
                        .world
                        .entities
                        .expect_ai_controller(self.target(), format_args!("returning checkpoint"))
                        .current_substate,
                    SeekingCharlySentToOfficer
                        | SeekingCharlyGoToOfficer
                        | SeekingCharlyGoToOfficerSeen
                ) {
                    self.face(self.target());
                    self.enemy_mut().base.clear_emoticon();

                    self.timer(20);
                } else {
                    self.duty();
                }
            }
            (SeekingOfficerWaitForCharly, CallCoordinate) => {
                if matches!(stimulus.info, StimulusInfo::Human(sender) if Some(sender) == self.enemy().base.antagonist)
                {
                    self.face(self.target());
                    self.say(Remark::OfficerRebukesCharly, SpeechFlags::MYTALK_1);
                    self.seek_state(SeekingOfficerLectureCharly);
                }
            }
            (SeekingOfficerLectureCharly, CallYourTalk1) => {
                self.say(Remark::OfficerRebukesCharlyEnd, SpeechFlags::MYTALK_2)
            }
            (SeekingOfficerLectureCharly, EventMyTalk2) => {
                let target = self.target();
                let ai = self.engine.ai(target, "checkpoint post");
                let position = if ai.has_patrol_path {
                    let path = ai
                        .patrol_path
                        .as_ref()
                        .map(|p| p.hiking_path_index)
                        .or(ai.detached_patrol_path_status.hiking_path_index)
                        .expect("checkpoint path must resolve")
                        .get() as usize;
                    let here = self.engine.live_ai_position(self.owner);
                    let points = &self.assets.navigation.hiking_paths[path].waypoints;
                    let mut best = Option::None;
                    let mut distance = u32::MAX as f32;
                    for (index, point) in points.iter().enumerate() {
                        let candidate = (point.x as f32 - here.x)
                            .abs()
                            .max((point.y as f32 - here.y).abs());
                        if candidate < distance {
                            best = Some(index);
                            distance = candidate;
                        }
                    }
                    let index = best.expect("checkpoint path requires a waypoint");
                    let point = &points[index];
                    Position {
                        x: point.x as f32,
                        y: point.y as f32,
                        sector: self.assets.navigation.hiking_waypoint_sector(
                            path,
                            index,
                            point.sector,
                        ),
                        level: point.level,
                    }
                } else {
                    ai.initial_position
                };
                self.engine
                    .duty_point_to(self.sim, self.assets, self.owner, position);
                self.seek_state(SeekingOfficerLectureCharlyPointing);
                self.say(Remark::OfficerEndsConversation, SpeechFlags::MYTALK_3);
                self.timer(20);
            }
            (SeekingOfficerLectureCharlyPointing, EventMyTalk3) => {
                self.engine.execute_ai_callback(
                    self.sim,
                    self.assets,
                    self.target(),
                    &Stimulus::new(CallYourTalk2),
                );
                self.duty();
            }
            (SeekingCharly, EventReachPoint) => {
                assert!(
                    !self.enemy().search_charly_way.is_empty(),
                    "checkpoint search arrival requires route"
                );
                self.enemy_mut().search_charly_way.remove(0);
                if self.enemy().search_charly_way.is_empty() {
                    if self.enemy().base.checkpoint_charly.is_none() {
                        self.duty();
                    } else {
                        self.seek_state(SeekingCharlyWatching);
                        self.engine.execute_ai_look_sidewards(
                            self.sim,
                            self.assets,
                            self.owner,
                            crate::ai::LookDirection::LeftRight,
                        );
                    }
                } else {
                    let point = self.enemy().search_charly_way[0];
                    let flags = GotoFlags::RUN
                        | if self.enemy().search_charly_way.len() > 1 {
                            GotoFlags::DONT_STOP
                        } else {
                            GotoFlags::empty()
                        };
                    self.engine
                        .duty_go_to(self.sim, self.assets, self.owner, point, flags);
                }
            }
            (SeekingCharlyWatching, EventDone) => {
                self.engine
                    .execute_ai_missed_charly_alert(self.sim, self.assets, self.owner)
            }
            _ => {}
        }
    }
    fn report_back(&mut self, event: StimulusType) {
        if event == StimulusType::EventReachPoint {
            self.duty();
            return;
        }
        let officer = self.target();
        if self.engine.npc_is_detecting_human(
            self.assets,
            self.owner,
            officer,
            self.engine.control.frame_counter,
        ) {
            if self.engine.execute_ai_callback(
                self.sim,
                self.assets,
                officer,
                &Stimulus::with_human(StimulusType::CallMrOfficerIAmBack, self.owner.index()),
            ) {
                self.seek_state(Substate::SeekingCharlyGoToOfficerSeen);
                self.timer(10);
            } else {
                self.duty();
            }
        } else {
            self.engine
                .unalert_live_charly_seekers(self.sim, self.assets, self.owner, self.owner);
            self.timer(10);
        }
    }
}
