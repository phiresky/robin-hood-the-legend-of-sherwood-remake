//! Direct officer and checkpoint calls preserve their caller stack.
#[cfg(test)]
mod tests;
use super::*;
use crate::ai::{
    AiEntityHandle, AiState, DutyFlags, EmoticonType, GotoFlags, Position, Remark, SpeechFlags,
    Stimulus, StimulusInfo, Substate,
};
use crate::ai_enemy::{EnemyAi, SeekFlags};
use crate::profiles::ProfileRank;
use crate::sim_rng::SimulationContext;

struct OfficerRpc<'a> {
    engine: &'a mut EngineInner,
    sim: &'a SimulationContext,
    assets: &'a LevelAssets,
    owner: EntityId,
}

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
            OfficerRpc {
                engine: self,
                sim,
                assets,
                owner,
            }
            .charly_event(substate, stimulus);
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
            OfficerRpc {
                engine: self,
                sim,
                assets,
                owner,
            }
            .charly_event(substate, stimulus);
            return Some(false);
        }
        if self.observation_ai(owner).base.current_substate == Substate::SeekingCharlyGoToOfficer
            && matches!(
                event,
                StimulusType::EventTimer | StimulusType::EventReachPoint
            )
        {
            OfficerRpc {
                engine: self,
                sim,
                assets,
                owner,
            }
            .report_back(event);
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
        Some(
            OfficerRpc {
                engine: self,
                sim,
                assets,
                owner,
            }
            .event(event, target),
        )
    }
}

impl OfficerRpc<'_> {
    fn ai(&self) -> &EnemyAi {
        self.engine.observation_ai(self.owner)
    }
    fn ai_mut(&mut self) -> &mut EnemyAi {
        self.engine.observation_ai_mut(self.owner)
    }
    fn settle(&mut self) {
        self.engine
            .drain_direct_ai_owner_boundary(self.sim, self.owner, self.assets);
    }
    fn state(&mut self, substate: Substate) {
        self.engine.duty_set_state(
            self.sim,
            self.assets,
            self.owner,
            AiState::Seeking,
            substate,
        );
    }
    fn timer(&mut self, frames: u32) {
        self.engine.observation_timer(self.owner, frames);
    }
    fn duty(&mut self) {
        self.engine.execute_ai_return_to_duty(
            self.sim,
            self.assets,
            self.owner,
            DutyFlags::empty(),
        );
    }
    fn target(&self) -> EntityId {
        self.engine.expect_human_id_for_ai_handle(
            self.ai()
                .base
                .antagonist
                .expect("officer call requires an antagonist")
                .get(),
            "officer call antagonist",
        )
    }
    fn halt(&mut self) {
        self.ai_mut().base.outbox.actor.queue_halt();
        self.settle();
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
        self.ai_mut()
            .base
            .set_transient_emoticon(emoticon, 20, frame);
        self.settle();
    }
    fn say(&mut self, remark: Remark, flags: SpeechFlags) {
        self.engine.execute_ai_speech(
            self.sim,
            self.assets,
            self.owner,
            crate::ai::AiSpeechAttempt {
                remark,
                flags: flags.bits(),
            },
        );
    }
    fn priority(&self) -> bool {
        self.ai().base.blood_alcohol as i32 <= crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            && self.ai().has_the_new_task_priority()
    }
    fn rank(&self, target: EntityId) -> ProfileRank {
        self.engine
            .world
            .entities
            .expect_enemy_ai(target, format_args!("officer call rank"))
            .get_rank()
    }
    fn accept(&mut self, state: Substate) {
        self.state(state);
        self.timer(20);
        self.emoticon(EmoticonType::QuestionMark);
    }
    fn event(&mut self, event: StimulusType, target: EntityId) -> bool {
        use StimulusType::*;
        use Substate::*;
        match event {
            EventSeesSoldier => {
                if self.ai().base.current_state != AiState::Default
                    && !(self.ai().base.current_state == AiState::Seeking
                        && matches!(
                            self.ai().base.current_substate,
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
                match self.ai().get_rank() {
                    ProfileRank::Soldier => {
                        self.ai_mut().base.antagonist = Some(AiEntityHandle::new(target.index()));
                        if self.engine.execute_ai_callback(
                            self.sim,
                            self.assets,
                            target,
                            &Stimulus::with_human(CallAlert, self.owner.index()),
                        ) {
                            self.state(SeekingRunningToOfficerSeen);
                            self.say(Remark::CallsOfficer, SpeechFlags::MYTALK_0);
                            let officer = self.target();
                            let input = extract_exact_forecast_input(
                                self.engine,
                                self.engine.expect_entity(officer, "officer forecast"),
                                selected_actor_is_passing_door(
                                    &self.engine.orders.sequence_manager,
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
                        self.ai_mut().base.antagonist = Some(AiEntityHandle::new(target.index()));
                        if self.engine.can_call_ai_soldier(self.owner, target) {
                            self.face(self.target());
                            self.state(SeekingOfficerCallSoldier);
                            self.ai_mut()
                                .base
                                .outbox
                                .actor
                                .delete_detectable_type(crate::element::DetectableType::Friend);
                            self.settle();
                        }
                    }
                    ProfileRank::Knight | ProfileRank::None => {}
                }
            }
            CallGoToOfficer => {
                if self.ai().base.current_state != AiState::Default
                    && self.ai().base.current_substate != SleepingAwakening
                {
                    return false;
                }
                self.ai_mut().base.antagonist = Some(AiEntityHandle::new(target.index()));
                self.state(SeekingCharlySentToOfficer);
                self.ai_mut().base.set_emoticon(EmoticonType::None);
                self.settle();
                self.timer(30);
                self.ai_mut().reported_to_officer = true;
                return true;
            }
            CallHey => {
                self.ai_mut().base.antagonist = Some(AiEntityHandle::new(target.index()));
                if matches!(
                    self.engine.expect_entity(target, "officer hail"),
                    Entity::Civilian(_)
                ) {
                    tracing::warn!("civilian officer hail is unsupported");
                    return false;
                }
                let react = matches!(
                    self.ai().base.current_state,
                    AiState::Default | AiState::Wondering
                ) || self.ai().base.current_state == AiState::Seeking
                    && matches!(
                        self.ai().base.current_substate,
                        SeekingRunningToOfficer
                            | SeekingRunningToOfficerSeen
                            | SeekingHeardstepsReactiontime
                            | SeekingBodyReactiontime
                    );
                if !react || self.ai().get_rank() != ProfileRank::Soldier || !self.priority() {
                    return false;
                }
                self.ai_mut().current_task_priority = self.ai().new_task_priority;
                self.engine
                    .observation_stop(self.sim, self.assets, self.owner);
                assert_eq!(self.rank(self.target()), ProfileRank::Officer);
                self.face(self.target());
                self.accept(SeekingSoldierCalledByOfficer);
                return true;
            }
            CallAlert => {
                self.ai_mut().base.antagonist = Some(AiEntityHandle::new(target.index()));
                if matches!(
                    self.engine.expect_entity(target, "alert sender"),
                    Entity::Civilian(_)
                ) {
                    if self.ai().base.current_state != AiState::Default {
                        return false;
                    }
                    self.halt();
                    self.face(self.target());
                    self.accept(SeekingWaitForAlertingCivilian);
                    return true;
                }
                match self.ai().get_rank() {
                    ProfileRank::Soldier => {
                        let react = matches!(
                            self.ai().base.current_state,
                            AiState::Default | AiState::Wondering
                        ) || self.ai().base.current_state == AiState::Seeking
                            && matches!(
                                self.ai().base.current_substate,
                                SeekingSoldierGiveReportToOfficer
                                    | SeekingSoldierGiveAlertingReportToOfficerStart
                                    | SeekingSoldierGiveAlertingReportToOfficerPoint
                                    | SeekingSoldierGiveAlertingReportToOfficerEnd
                            );
                        if !react || !self.priority() {
                            return false;
                        }
                        self.halt();
                        self.ai_mut().current_task_priority = self.ai().new_task_priority;
                        self.ai_mut().gather_position_instructed = false;
                        self.ai_mut().base.friends_are_alerted = true;
                        assert_eq!(self.rank(self.target()), ProfileRank::Officer);
                        let position = self.engine.live_ai_position(self.target());
                        self.ai_mut().officers_position = position;
                        self.face_position(self.ai().officers_position);
                        self.accept(SeekingGroupCalledByOfficer);
                        return true;
                    }
                    ProfileRank::Officer => {
                        if !self.officer_ready() {
                            return false;
                        }
                        self.halt();
                        self.ai_mut().base.friends_are_alerted = true;
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
                self.ai_mut().base.antagonist = Some(AiEntityHandle::new(target.index()));
                if self.ai().base.current_state == AiState::Seeking
                    && self.ai().base.current_substate == SeekingOfficerWaitForCharly
                {
                    return true;
                }
                if !self.officer_ready() {
                    return false;
                }
                self.halt();
                assert_eq!(self.rank(self.target()), ProfileRank::Soldier);
                self.face(self.target());
                self.state(SeekingOfficerWaitForCharly);
                self.say(Remark::FoundCharly, SpeechFlags::empty());
                self.timer(20);
                self.emoticon(EmoticonType::XMark);
                return true;
            }
            CallCharlyIsBack => {
                let s = self.ai().base.current_substate;
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
                    if self.ai().base.my_reconnaissance_report.charly
                        == Some(AiEntityHandle::new(target.index()))
                    {
                        self.ai_mut().base.set_checkpoint_charly(Option::None);
                        self.settle();
                        self.face(target);
                        self.ai_mut().base.clear_emoticon();
                        self.settle();
                        self.ai_mut()
                            .seek_flags
                            .remove(SeekFlags::REPORT_OFFICER_AFTER);
                        self.state(SeekingLookingResurrectedCharly);
                        let human = self.engine.expect_entity(target, "returned checkpoint");
                        self.timer(if human.is_dead() || human.is_unconscious() {
                            200
                        } else {
                            20
                        });
                    }
                } else {
                    self.ai_mut().base.set_checkpoint_charly(Option::None);
                    self.settle();
                }
            }
            _ => unreachable!(),
        }
        false
    }
    fn officer_ready(&self) -> bool {
        self.ai().base.current_state == AiState::Default
            || self.ai().base.current_state == AiState::Seeking
                && matches!(
                    self.ai().base.current_substate,
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
                if draw < u32::from(self.ai().base.sorrow_level) + 10 {
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
                    self.ai_mut().base.outbox.actor.look_sidewards = Some(direction);
                    self.settle();
                }
                self.ai_mut().base.sorrow_level = self
                    .ai()
                    .base
                    .sorrow_level
                    .wrapping_add(self.ai().base.delta_sorrow_level);
                if self.ai().base.sorrow_level > 1000 {
                    self.ai_mut().base.sorrow_level = 0;
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
                if self.ai().base.macro_in_progress {
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
                    .ai()
                    .base
                    .synchronize_charly
                    .expect("synchronization timer requires partner");
                let partner = self
                    .engine
                    .expect_human_id_for_ai_handle(partner.get(), "synchronization partner");
                if self
                    .engine
                    .world
                    .entities
                    .expect_ai_controller(partner, format_args!("synchronization partner"))
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
                if matches!(stimulus.info, StimulusInfo::Index(index) if index == self.ai().base.synchronize_index)
                {
                    assert!(
                        self.ai().base.macro_in_progress,
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
                self.ai_mut().base.my_reconnaissance_report.charly_seen = true;
                if self.ai().get_rank() == ProfileRank::Officer && !self.ai().alerted_us.is_empty()
                {
                    let state = self.ai().previous_state.get("previous state");
                    let substate = self.ai().previous_substate.get("previous substate");
                    self.engine
                        .duty_set_state(self.sim, self.assets, self.owner, state, substate);
                    self.timer(10);
                } else {
                    self.duty();
                }
            }
            (SeekingSendCharlyToOfficer, EventMyTalk1) => {
                if let Some(friend) = self.ai().base.friend_in_trouble {
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
                            self.ai().officers_position,
                        );
                    }
                } else {
                    self.duty();
                }
            }
            (SeekingSendCharlyToOfficer, EventMyTalk2) => {
                self.state(SeekingLookingResurrectedCharly);
                let friend = self
                    .ai()
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
                self.state(SeekingCharlyGoToOfficer);
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
                    .world
                    .entities
                    .expect_ai_controller(self.target(), format_args!("checkpoint officer"))
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
                self.state(SeekingCharlyGetLectureByOfficer);
            }
            (SeekingCharlyGetLectureByOfficer, CallYourTalk1) => {
                self.say(Remark::CharlyDefendsHimself, SpeechFlags::MYTALK_1);
                self.state(SeekingCharlyGetLectureByOfficer2);
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
                    self.ai_mut().base.clear_emoticon();
                    self.settle();
                    self.timer(20);
                } else {
                    self.duty();
                }
            }
            (SeekingOfficerWaitForCharly, CallCoordinate) => {
                if matches!(stimulus.info, StimulusInfo::Human(sender) if Some(sender) == self.ai().base.antagonist)
                {
                    self.face(self.target());
                    self.say(Remark::OfficerRebukesCharly, SpeechFlags::MYTALK_1);
                    self.state(SeekingOfficerLectureCharly);
                }
            }
            (SeekingOfficerLectureCharly, CallYourTalk1) => {
                self.say(Remark::OfficerRebukesCharlyEnd, SpeechFlags::MYTALK_2)
            }
            (SeekingOfficerLectureCharly, EventMyTalk2) => {
                let target = self.target();
                let ai = self
                    .engine
                    .world
                    .entities
                    .expect_ai_controller(target, format_args!("checkpoint post"));
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
                self.state(SeekingOfficerLectureCharlyPointing);
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
                    !self.ai().search_charly_way.is_empty(),
                    "checkpoint search arrival requires route"
                );
                self.ai_mut().search_charly_way.remove(0);
                if self.ai().search_charly_way.is_empty() {
                    if self.ai().base.checkpoint_charly.is_none() {
                        self.duty();
                    } else {
                        self.state(SeekingCharlyWatching);
                        self.ai_mut().base.outbox.actor.look_sidewards =
                            Some(crate::ai::LookDirection::LeftRight);
                        self.settle();
                    }
                } else {
                    let point = self.ai().search_charly_way[0];
                    let flags = GotoFlags::RUN
                        | if self.ai().search_charly_way.len() > 1 {
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
                self.state(Substate::SeekingCharlyGoToOfficerSeen);
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
