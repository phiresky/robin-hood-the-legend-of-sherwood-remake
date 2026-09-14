//! Live officer rendezvous and instruction conversations.

use super::*;
use crate::ai::{
    AiEntityHandle, AiState, DutyFlags, GotoFlags, Position, Remark, ReportType, SpeechFlags,
    Stimulus, StimulusInfo, Substate,
};
use crate::ai_enemy::{EnemyAi, SeekFlags, task_priority};
use crate::element::{Element as _, Human as _};
use crate::parameters_ai;
use crate::sim_rng::SimulationContext;

struct Rendezvous<'a> {
    engine: &'a mut EngineInner,
    sim: &'a SimulationContext,
    assets: &'a LevelAssets,
    owner: EntityId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::test_support::actors::make_test_ai_soldier;

    #[test]
    fn waiting_group_stops_pruning_at_first_approaching_member() {
        let mut engine = EngineInner::new();
        let [owner, obsolete, approaching, later_obsolete] = std::array::from_fn(|_| {
            engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists))
        });
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        let officer = engine.seek_enemy_mut(owner);
        officer.base.current_state = AiState::Seeking;
        officer.base.current_substate = Substate::SeekingOfficerWaitForGroup;
        officer.alerted_us = vec![
            obsolete.index(),
            approaching.index(),
            later_obsolete.index(),
        ];
        let member = engine.seek_enemy_mut(approaching);
        member.base.current_state = AiState::Seeking;
        member.base.current_substate = Substate::SeekingGroupGoToOfficer;
        let result = engine.execute_ai_officer_rendezvous_event(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &Stimulus::new(StimulusType::EventTimer),
        );
        assert_eq!(result, Some(false));
        assert_eq!(
            engine.seek_enemy(owner).alerted_us,
            vec![approaching.index(), later_obsolete.index()]
        );
        assert_eq!(
            engine.seek_enemy(owner).base.current_substate,
            Substate::SeekingOfficerWaitForGroup
        );
    }

    #[test]
    fn officer_rendezvous_leaves_interruptions_to_event_dispatch() {
        let mut engine = EngineInner::new();
        let owner =
            engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.seek_enemy_mut(owner).base.current_state = AiState::Seeking;
        engine.seek_enemy_mut(owner).base.current_substate =
            Substate::SeekingOfficerWaitForInstructedSoldier;
        for event in [
            StimulusType::EventView,
            StimulusType::EventCouldntReachPoint,
            StimulusType::CallReport,
        ] {
            assert_eq!(
                engine.execute_ai_officer_rendezvous_event(
                    &crate::sim_rng::test_context(),
                    &assets,
                    owner,
                    &Stimulus::new(event)
                ),
                None
            );
        }
    }
}

impl EngineInner {
    pub(in crate::engine) fn execute_ai_officer_rendezvous_event(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        let substate = self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("officer rendezvous"))
            .current_substate;
        use Substate::*;
        if !matches!(
            stimulus.stimulus_type,
            StimulusType::EventReachPoint
                | StimulusType::EventDone
                | StimulusType::EventTimer
                | StimulusType::EventSyncCharly
                | StimulusType::CallCoordinate
                | StimulusType::CallInstruction
                | StimulusType::CallReport
                | StimulusType::EventGaloppLoopEnd
                | StimulusType::EventMyTalk0
                | StimulusType::EventMyTalk1
                | StimulusType::EventMyTalk2
                | StimulusType::EventMyTalk3
                | StimulusType::CallYourTalk0
                | StimulusType::CallYourTalk1
                | StimulusType::CallYourTalk2
                | StimulusType::CallYourTalk3
        ) {
            return Option::None;
        }
        if !matches!(
            substate,
            SeekingOfficerWaitForSoldier
                | SeekingOfficerInstructSoldier
                | SeekingOfficerWaitForInstructedSoldier
                | SeekingSoldierCalledByOfficer
                | SeekingSoldierGoToOfficer
                | SeekingSoldierGetInstructedByOfficer
                | SeekingOfficerCallGroup
                | SeekingOfficerWaitForGroup
                | SeekingOfficerInstructGroup
                | SeekingOfficerInstructGroupPointing
                | SeekingOfficerWaitForInstructedGroup
                | SeekingOfficerWaitInsideHouseToInstructGroup
                | SeekingOfficerLeavingHouseToInstructGroup
                | SeekingGroupCalledByOfficer
                | SeekingGroupGoToOfficer
                | SeekingGroupGetInstructedByOfficer
        ) || matches!(
            substate,
            SeekingOfficerWaitForInstructedSoldier | SeekingOfficerWaitForInstructedGroup
        ) && stimulus.stimulus_type == StimulusType::CallReport
        {
            return Option::None;
        }
        Some(
            Rendezvous {
                engine: self,
                sim,
                assets,
                owner,
            }
            .event(substate, stimulus),
        )
    }
}

impl Rendezvous<'_> {
    fn enemy(&self) -> &EnemyAi {
        self.engine.seek_enemy(self.owner)
    }
    fn enemy_mut(&mut self) -> &mut EnemyAi {
        self.engine.seek_enemy_mut(self.owner)
    }
    fn target(&self) -> EntityId {
        let handle = self
            .enemy()
            .base
            .antagonist
            .expect("officer rendezvous requires an antagonist");
        self.engine
            .expect_human_id_for_ai_handle(handle.get(), "officer rendezvous antagonist")
    }
    fn substate(&self, target: EntityId) -> Substate {
        self.engine
            .world
            .entities
            .expect_ai_controller(target, format_args!("officer rendezvous target"))
            .current_substate
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
        let frame = self.engine.control.frame_counter;
        self.enemy_mut().base.launch_timer(frames, frame);
    }
    fn duty(&mut self) {
        self.engine.execute_ai_return_to_duty(
            self.sim,
            self.assets,
            self.owner,
            DutyFlags::empty(),
        );
    }
    fn say(&mut self, remark: Remark, flags: SpeechFlags) {
        self.enemy_mut().base.say_with_flags(remark, flags);
        self.engine
            .drain_direct_ai_owner_boundary(self.sim, self.owner, self.assets);
    }
    fn call(&mut self, target: EntityId, kind: StimulusType, human: bool) {
        let mut stimulus = Stimulus::new(kind);
        if human {
            stimulus.info = StimulusInfo::Human(AiEntityHandle::new(self.owner.index()));
        }
        self.engine
            .execute_ai_callback(self.sim, self.assets, target, &stimulus);
    }
    fn face_target(&mut self) {
        let target = self.target();
        let position = self.engine.live_ai_position(target);
        let elevation = self
            .engine
            .expect_entity(target, "officer facing")
            .position_iface()
            .get_elevation() as i16 as f32;
        self.engine.duty_face_position_at_elevation(
            self.sim,
            self.assets,
            self.owner,
            position,
            elevation,
        );
    }
    fn point(&mut self, position: Position) {
        self.engine
            .duty_point_to(self.sim, self.assets, self.owner, position);
    }
    fn seek(&mut self, point: Position, radius: u16, flags: SeekFlags) {
        self.engine.execute_ai_seek_area(
            self.sim,
            self.assets,
            self.owner,
            point,
            radius,
            flags,
            crate::ai_enemy::UNDEFINED_DIRECTION,
        );
    }
    fn group_officer_waiting(&self) -> bool {
        matches!(
            self.substate(self.target()),
            Substate::SeekingOfficerWaitForGroup
                | Substate::SeekingDetectedCharly
                | Substate::SeekingOfficerWaitInsideHouseToInstructGroup
                | Substate::SeekingOfficerLeavingHouseToInstructGroup
        )
    }
    fn event(&mut self, substate: Substate, stimulus: &Stimulus) -> bool {
        use StimulusType::*;
        use Substate::*;
        let event = stimulus.stimulus_type;
        match substate {
            SeekingOfficerWaitForSoldier => match event {
                EventTimer => {
                    if matches!(
                        self.substate(self.target()),
                        SeekingSoldierCalledByOfficer | SeekingSoldierGoToOfficer
                    ) {
                        self.face_target();
                        self.timer(20);
                    } else {
                        self.duty();
                    }
                }
                CallCoordinate => {
                    self.state(SeekingOfficerInstructSoldier);
                    self.point(self.enemy().base.alert_soldiers_point);
                    self.timer(20);
                }
                _ => {}
            },
            SeekingOfficerInstructSoldier => match event {
                CallYourTalk1 => self.say(Remark::OfficerSendsOutSoldier, SpeechFlags::MYTALK_1),
                EventMyTalk1 => self.call(self.target(), CallYourTalk1, false),
                CallYourTalk2 => {
                    self.state(SeekingOfficerWaitForInstructedSoldier);
                    self.enemy_mut().missed_soldier_timer = 0;
                    self.timer(30);
                }
                EventTimer => {
                    if self.substate(self.target()) == SeekingSoldierGetInstructedByOfficer {
                        self.timer(20);
                    } else {
                        self.duty();
                    }
                }
                _ => {}
            },
            SeekingOfficerWaitForInstructedSoldier => match event {
                CallYourTalk1 => self.say(Remark::OfficerAsksWhatsup, SpeechFlags::empty()),
                EventTimer => {
                    let target = self.target();
                    let visible = self
                        .engine
                        .live_ai_detects_180(self.assets, self.owner, target);
                    let target_entity = self.engine.expect_entity(target, "instructed soldier");
                    if visible
                        && target_entity.human_life_points() > 0
                        && !target_entity.is_unconscious()
                    {
                        if self
                            .engine
                            .world
                            .entities
                            .expect_ai_controller(target, format_args!("instructed soldier state"))
                            .current_state
                            == AiState::Seeking
                        {
                            self.enemy_mut().missed_soldier_timer = 0;
                            self.timer(30);
                        } else {
                            self.duty();
                        }
                    } else {
                        self.enemy_mut().missed_soldier_timer =
                            self.enemy().missed_soldier_timer.wrapping_add(1);
                        if self.enemy().missed_soldier_timer > 100 {
                            let position = self.engine.live_ai_position(self.owner);
                            if !self.engine.execute_ai_alert_soldiers(
                                self.sim,
                                self.assets,
                                self.owner,
                                position,
                                0,
                            ) {
                                let position = self.engine.live_ai_position(self.owner);
                                self.seek(
                                    position,
                                    parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                                    SeekFlags::LOCATION_FIRST | self.enemy().seek_flags,
                                );
                            }
                        }
                    }
                }
                _ => {}
            },
            SeekingSoldierCalledByOfficer if event == EventTimer => {
                let position = self.engine.live_ai_position(self.target());
                self.engine.duty_go_near(
                    self.sim,
                    self.assets,
                    self.owner,
                    position,
                    40,
                    GotoFlags::empty(),
                );
                self.state(SeekingSoldierGoToOfficer);
                self.timer(20);
            }
            SeekingSoldierGoToOfficer => match event {
                EventTimer => {
                    if self.substate(self.target()) == SeekingOfficerWaitForSoldier {
                        self.timer(20);
                    } else {
                        self.duty();
                    }
                }
                EventReachPoint => {
                    if self.substate(self.target()) == SeekingOfficerWaitForSoldier {
                        self.call(self.target(), CallCoordinate, true);
                        self.state(SeekingSoldierGetInstructedByOfficer);
                        self.timer(20);
                        self.say(Remark::AwaitsOrders, SpeechFlags::MYTALK_1);
                    } else {
                        self.duty();
                    }
                }
                _ => {}
            },
            SeekingSoldierGetInstructedByOfficer => match event {
                EventMyTalk1 => self.call(self.target(), CallYourTalk1, false),
                CallYourTalk1 => {
                    let body = self
                        .engine
                        .world
                        .entities
                        .expect_ai_controller(self.target(), format_args!("officer selected body"))
                        .detected_body;
                    if body
                        .is_some_and(|body| self.enemy().already_seen_bodies.contains(&body.get()))
                    {
                        self.call(self.target(), CallYourTalk2, false);
                        self.state(SeekingSoldierReturnToOfficer);
                        self.call(self.owner, EventReachPoint, false);
                    } else {
                        self.say(Remark::GiveOrReceiveOrder, SpeechFlags::MYTALK_2);
                    }
                }
                EventMyTalk2 => {
                    let body = self
                        .engine
                        .world
                        .entities
                        .expect_ai_controller(self.target(), format_args!("officer selected body"))
                        .detected_body;
                    self.call(self.target(), CallYourTalk2, false);
                    if let Some(body) = body {
                        let body = self
                            .engine
                            .expect_human_id_for_ai_handle(body.get(), "instructed body");
                        self.enemy_mut()
                            .base
                            .outbox
                            .actor
                            .add_detectable((body, crate::element::DetectableType::Body));
                        self.engine.drain_direct_ai_owner_boundary(
                            self.sim,
                            self.owner,
                            self.assets,
                        );
                    }
                    self.enemy_mut().current_task_priority = task_priority::SEEKING;
                    let point = self
                        .engine
                        .world
                        .entities
                        .expect_ai_controller(
                            self.target(),
                            format_args!("officer instruction point"),
                        )
                        .alert_soldiers_point;
                    self.enemy_mut().base.alert_soldiers_point = point;
                    let position = self.engine.live_ai_position(self.target());
                    self.enemy_mut().officers_position = position;
                    self.seek(
                        point,
                        0,
                        SeekFlags::LOCATION_FIRST | SeekFlags::REPORT_OFFICER_AFTER,
                    );
                }
                EventTimer => {
                    if self.substate(self.target()) == SeekingOfficerInstructSoldier {
                        self.timer(20);
                    } else {
                        self.duty();
                    }
                }
                _ => {}
            },
            SeekingOfficerCallGroup if event == EventTimer => {
                if !self.engine.execute_ai_alert_soldiers(
                    self.sim,
                    self.assets,
                    self.owner,
                    self.enemy().base.seek_position,
                    self.enemy().seek_flags.bits(),
                ) {
                    self.duty();
                }
            }
            SeekingOfficerWaitForGroup if matches!(event, CallCoordinate | EventTimer) => {
                let mut index = 0;
                let mut count = self.enemy().alerted_us.len();
                while index < count {
                    let target = self.engine.expect_human_id_for_ai_handle(
                        self.enemy().alerted_us[index],
                        "waiting group member",
                    );
                    match self.substate(target) {
                        SeekingGroupCalledByOfficer | SeekingGroupGoToOfficer => return false,
                        SeekingGroupGetInstructedByOfficer => index += 1,
                        _ => {
                            self.enemy_mut().alerted_us.remove(index);
                            count -= 1;
                        }
                    }
                }
                if count > 0 {
                    self.state(SeekingOfficerInstructGroup);
                    self.timer(10);
                } else {
                    self.duty();
                }
            }
            SeekingOfficerInstructGroup if event == EventTimer => {
                self.state(SeekingOfficerInstructGroupPointing);
                let remark = if self.enemy().base.my_reconnaissance_report.report_type
                    == ReportType::MissedCharly
                {
                    Remark::OfficerSendsOutGroupForCharly
                } else {
                    Remark::OfficerSendsOutGroup
                };
                self.say(remark, SpeechFlags::empty());
                self.point(self.enemy().base.seek_position);
            }
            SeekingOfficerInstructGroupPointing if event == EventDone => self
                .engine
                .execute_ai_officer_instruct_group(self.sim, self.assets, self.owner),
            SeekingOfficerWaitForInstructedGroup if event == EventTimer => {
                self.wait_for_instructed_group()
            }
            SeekingOfficerWaitInsideHouseToInstructGroup if event == EventTimer => {
                self.state(SeekingOfficerLeavingHouseToInstructGroup);
                self.engine.duty_go_to(
                    self.sim,
                    self.assets,
                    self.owner,
                    self.enemy().gather_position,
                    GotoFlags::empty(),
                );
            }
            SeekingOfficerLeavingHouseToInstructGroup => match event {
                EventReachPoint => self.engine.duty_face_direction(
                    self.sim,
                    self.assets,
                    self.owner,
                    self.enemy().gather_direction,
                ),
                EventDone => {
                    self.state(SeekingOfficerWaitForGroup);
                    self.timer(1);
                }
                _ => {}
            },
            SeekingGroupCalledByOfficer if event == EventTimer => {
                if self.enemy().gather_position_instructed {
                    self.engine.duty_go_to(
                        self.sim,
                        self.assets,
                        self.owner,
                        self.enemy().gather_position,
                        GotoFlags::RUN,
                    );
                } else {
                    let position = self.engine.live_ai_position(self.target());
                    self.engine.duty_go_near(
                        self.sim,
                        self.assets,
                        self.owner,
                        position,
                        parameters_ai::AI_TALK_DISTANCE,
                        GotoFlags::RUN,
                    );
                }
                self.state(SeekingGroupGoToOfficer);
                self.timer(20);
            }
            SeekingGroupGoToOfficer => match event {
                EventTimer => {
                    if self.group_officer_waiting() {
                        self.timer(20);
                    } else {
                        self.duty();
                    }
                }
                EventReachPoint => {
                    if self.enemy().gather_position_instructed {
                        self.engine.duty_face_direction(
                            self.sim,
                            self.assets,
                            self.owner,
                            self.enemy().gather_direction,
                        );
                    } else {
                        self.face_target();
                    }
                }
                EventDone => {
                    if self.group_officer_waiting() {
                        self.state(SeekingGroupGetInstructedByOfficer);
                        self.call(self.target(), CallCoordinate, true);
                    } else {
                        self.duty();
                    }
                }
                _ => {}
            },
            SeekingGroupGetInstructedByOfficer if event == CallInstruction => {
                let StimulusInfo::Hint(hint) = &stimulus.info else {
                    panic!("group instruction requires a hint");
                };
                self.enemy_mut().base.alert_soldiers_point = hint.seek_point;
                let officer = self
                    .engine
                    .expect_human_id_for_ai_handle(hint.who_tells_me.get(), "instructing officer");
                let position = self.engine.live_ai_position(officer);
                self.enemy_mut().officers_position = position;
                self.engine
                    .ai
                    .global
                    .forbidden_remarks
                    .push(crate::ai::ForbiddenRemark {
                        remark: Remark::TellsOfficerNothing,
                        flags: crate::ai::RemarkTargetFlags::THIS_GUY.bits(),
                        speech_id: 0,
                        guy_index: self.engine.world.original_creation_order(self.owner) as u16,
                        bad_guy: true,
                        forbidden_till_frame: self.engine.control.frame_counter + 30,
                    });
                self.seek(
                    hint.seek_point,
                    parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                    SeekFlags::from_bits_truncate(hint.seek_flags),
                );
                return true;
            }
            _ => {}
        }
        false
    }

    fn wait_for_instructed_group(&mut self) {
        use Substate::*;
        let mut index = 0;
        let mut count = self.enemy().alerted_us.len();
        while index < count {
            let target = self.engine.expect_human_id_for_ai_handle(
                self.enemy().alerted_us[index],
                "instructed group member",
            );
            if matches!(
                self.substate(target),
                SeekingSeekpoint
                    | SeekingSeekpointWatching
                    | SeekingSeekpointWatchingSidewards
                    | SeekingSeekpointPassedAmbushPointLeft
                    | SeekingSeekpointPassedAmbushPointRight
                    | SeekingSeekpointCheckingAmbushPoint
                    | SeekingSeekpointApproachingBeggar
                    | SeekingSeekpointIdentifyingBeggar1
                    | SeekingSeekpointIdentifyingBeggar2
                    | SeekingSoldierReturnToOfficer
                    | SeekingSoldierGiveReportToOfficer
                    | SeekingRunningToOfficer
                    | SeekingRunningToOfficerSeen
                    | SeekingBodyReactiontime
                    | SeekingBody
                    | SeekingNet
                    | SeekingBodyLookingDeadBody
                    | SeekingBodyAwakeningSleeperr
                    | SeekingDetectedCharly
            ) {
                index += 1;
            } else {
                self.enemy_mut().alerted_us.remove(index);
                count -= 1;
            }
        }
        if count > 0 {
            self.timer(30);
            return;
        }
        let report = &self.enemy().base.my_reconnaissance_report;
        if report.report_type == ReportType::MissedCharly
            && let Some(charly) = report.charly
        {
            let charly = self
                .engine
                .expect_human_id_for_ai_handle(charly.get(), "returning checkpoint");
            if matches!(
                self.substate(charly),
                SeekingCharlySentToOfficer | SeekingCharlyGoToOfficer
            ) {
                self.timer(30);
                return;
            }
        }
        self.duty();
    }
}
