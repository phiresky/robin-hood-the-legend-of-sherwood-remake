//! Live officer rendezvous and instruction conversations.

use super::*;
use crate::ai::{
    AiEntityHandle, AiState, DutyFlags, GotoFlags, Position, Remark, ReportType, SpeechFlags,
    Stimulus, StimulusInfo, Substate,
};
use crate::ai_enemy::{EnemyAi, SeekFlags, task_priority};
use crate::parameters_ai;
use crate::profiles::ProfileRank;
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

    #[test]
    fn civilian_report_seeks_report_location_when_no_officer_can_accept() {
        let mut engine = EngineInner::new();
        let sector = crate::engine::test_support::ensure_ordinary_sector(&mut engine, 1, 0);
        let mut soldier = make_test_ai_soldier(crate::element::Camp::Lacklandists);
        soldier.element_data_mut().set_sector(Some(sector));
        soldier
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(100.0, 100.0, 0.0));
        let owner = engine.add_test_entity(soldier);
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        let report = Position {
            x: 200.0,
            y: 100.0,
            sector: Some(sector),
            level: 0,
        };
        let ai = engine.seek_enemy_mut(owner);
        crate::engine::test_support::actors::edit_enemy_profile(&mut assets, ai, |profile| {
            profile.rank = ProfileRank::Soldier
        });
        crate::engine::test_support::actors::edit_enemy_profile(&mut assets, ai, |profile| {
            profile.initiative = 0
        });
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingGetAlertingReportFromCivilianLook;
        ai.base.seek_position = report;
        assert_eq!(
            engine.execute_ai_officer_rendezvous_event(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                &Stimulus::new(StimulusType::EventTimer),
            ),
            Some(false)
        );
        let ai = engine.seek_enemy(owner);
        assert_eq!(ai.seek_center, report);
        assert_eq!(ai.personal_seek_point_1.as_ref().unwrap().position, report);
        assert!(ai.seek_flags.contains(SeekFlags::LOCATION_FIRST));
        assert!(!ai.seek_flags.contains(SeekFlags::LOOK_FOR_HELP_AFTER));
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
            SeekingOfficerCallSoldier
                | SeekingRunningToOfficer
                | SeekingWaitForAlertingCivilian
                | SeekingGetReportFromCivilian
                | SeekingGetAlertingReportFromCivilian
                | SeekingGetAlertingReportFromCivilianLook
                | SeekingOfficerWaitForAlertingSoldier
                | SeekingOfficerGetAlertingReportFromSoldier
                | SeekingOfficerWaitForSoldier
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
            SeekingOfficerWaitForInstructedSoldier
                | SeekingOfficerWaitForInstructedGroup
                | SeekingOfficerWaitForAlertingSoldier
                | SeekingWaitForAlertingCivilian
        ) && stimulus.stimulus_type == StimulusType::CallReport
        {
            return Option::None;
        }
        if substate == SeekingOfficerGetAlertingReportFromSoldier
            && matches!(
                stimulus.stimulus_type,
                StimulusType::CallYourTalk1 | StimulusType::EventMyTalk1
            )
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
            SeekingOfficerCallSoldier if event == EventDone => {
                let mut call = Stimulus::new(CallHey);
                call.info = StimulusInfo::Human(AiEntityHandle::new(self.owner.index()));
                let target = self.target();
                assert!(
                    self.engine
                        .expect_entity(target, "called soldier")
                        .enemy_ai()
                        .is_some(),
                    "officer call requires enemy-soldier target {target:?}"
                );
                if self
                    .engine
                    .execute_ai_callback(self.sim, self.assets, target, &call)
                {
                    self.state(SeekingOfficerWaitForSoldier);
                    let frame = self.engine.control.frame_counter;
                    self.enemy_mut().base.set_transient_emoticon(
                        crate::ai::EmoticonType::XMark,
                        20,
                        frame,
                    );

                    self.say(Remark::OfficerCallsSoldier, SpeechFlags::empty());
                    self.timer(20);
                } else {
                    self.duty();
                }
            }
            SeekingRunningToOfficer => self.running_to_officer(event),
            SeekingWaitForAlertingCivilian if event == EventTimer => {
                if matches!(
                    self.substate(self.target()),
                    SeekingCivilianRunningToSoldierSeen
                        | SeekingCivilianGiveAlertingReportToSoldierStart
                        | SeekingCivilianGiveAlertingReportToSoldierPoint
                        | SeekingCivilianGiveAlertingReportToSoldierEnd
                ) {
                    self.face_target();
                    self.timer(20);
                } else {
                    self.duty();
                }
            }
            SeekingGetReportFromCivilian if event == EventTimer => self.duty(),
            SeekingGetAlertingReportFromCivilian if event == EventTimer => {
                let point = self.enemy().base.seek_position;
                let point = self.engine.position_to_point_3d(
                    self.assets,
                    point.sector,
                    point.level,
                    point.x,
                    point.y,
                );
                let body = self
                    .engine
                    .expect_entity(self.owner, "civilian report listener")
                    .element_data()
                    .position();
                let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                    point.x - body.x,
                    point.y - body.y,
                );
                self.engine.duty_face_direction(
                    self.sim,
                    self.assets,
                    self.owner,
                    direction as u16,
                );
                self.state(SeekingGetAlertingReportFromCivilianLook);
                self.timer(30);
            }
            SeekingGetAlertingReportFromCivilianLook if event == EventTimer => {
                self.act_on_civilian_report()
            }
            SeekingOfficerWaitForAlertingSoldier => match event {
                CallYourTalk0 => self.say(Remark::OfficerAsksWhatsup, SpeechFlags::empty()),
                EventTimer => {
                    if matches!(
                        self.substate(self.target()),
                        SeekingRunningToOfficerSeen
                            | SeekingSoldierGiveAlertingReportToOfficerStart
                            | SeekingSoldierGiveAlertingReportToOfficerPoint
                            | SeekingSoldierGiveAlertingReportToOfficerEnd
                    ) {
                        self.face_target();
                        self.timer(20);
                    } else {
                        self.duty();
                    }
                }
                _ => {}
            },
            SeekingOfficerGetAlertingReportFromSoldier if event == EventTimer => {
                if !self.engine.execute_ai_alert_soldiers(
                    self.sim,
                    self.assets,
                    self.owner,
                    self.enemy().base.seek_position,
                    0,
                ) {
                    self.duty();
                }
            }
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
                        let already_detectable = self
                            .engine
                            .world
                            .entities
                            .expect_ai_actor_data(self.owner, format_args!("instructed soldier"))
                            .detectable_lists
                            [crate::element::DetectableType::Body as usize]
                            .iter()
                            .any(|entry| entry.element == Some(body));
                        if !already_detectable {
                            self.engine.execute_ai_add_detectable(
                                self.owner,
                                body,
                                crate::element::DetectableType::Body,
                            );
                        }
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

    fn forecast_officer(&mut self) {
        let target = self.target();
        let input = extract_exact_forecast_input(
            self.engine,
            self.engine
                .expect_entity(target, "officer destination forecast"),
            selected_actor_is_passing_door(
                &self.engine.world.entities,
                &self.engine.orders.sequence_manager,
                target,
            ),
        )
        .expect("officer forecast requires an actor");
        let destination = crate::ai::prepare_forecast_destination_for_ia(
            &input,
            &self.engine.script_domains.interactables.doors,
            &self.engine.world.fast_grid.level.sectors,
            &self.engine.world.fast_grid.level.sector_number_map,
        )
        .resolve(self.sim)
        .position;
        self.enemy_mut().gather_position = destination;
        self.engine.duty_go_near(
            self.sim,
            self.assets,
            self.owner,
            destination,
            parameters_ai::AI_TALK_DISTANCE,
            GotoFlags::RUN,
        );
    }

    fn running_to_officer(&mut self, event: StimulusType) {
        match event {
            StimulusType::EventTimer => {
                let position = self.engine.live_ai_position(self.target());
                let gather = self.enemy().gather_position;
                let dx = position.x - gather.x;
                let dy = position.y - gather.y;
                if dx * dx + dy * dy
                    > (parameters_ai::AI_TALK_DISTANCE * parameters_ai::AI_TALK_DISTANCE) as f32
                {
                    self.forecast_officer();
                }
                self.timer(50);
            }
            StimulusType::EventReachPoint => {
                let target = self.target();
                let officer = self
                    .engine
                    .world
                    .entities
                    .expect_ai_controller(target, format_args!("officer arrival"));
                if officer.current_state == AiState::Default
                    || matches!(
                        officer.current_substate,
                        Substate::SeekingOfficerWaitForInstructedSoldier
                            | Substate::SeekingDetectedCharly
                            | Substate::SeekingOfficerWaitForInstructedGroup
                    )
                {
                    let officer = self.engine.live_ai_position(target);
                    let here = self.engine.live_ai_position(self.owner);
                    let dx = officer.x - here.x;
                    let dy = officer.y - here.y;
                    if dx * dx + dy * dy
                        > (parameters_ai::AI_TALK_DISTANCE * parameters_ai::AI_TALK_DISTANCE) as f32
                    {
                        self.forecast_officer();
                    } else {
                        self.engine.execute_ai_delete_detectable_type(
                            self.owner,
                            crate::element::DetectableType::Friend,
                        );

                        self.state(Substate::SeekingRunningToOfficerSeen);
                        self.call(self.owner, StimulusType::EventReachPoint, false);
                    }
                } else if !self
                    .engine
                    .execute_ai_alert_officer(self.sim, self.assets, self.owner)
                {
                    self.duty();
                }
            }
            _ => {}
        }
    }

    fn seek_before_alert(&self) -> bool {
        if self.enemy().base.blood_alcohol as i32 > parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT {
            return false;
        }
        let entity = self.engine.expect_entity(self.owner, "report initiative");
        if !entity.is_active()
            || self
                .engine
                .entity_data_in_building_sector(entity.element_data())
        {
            tracing::trace!("indoor initiative question falls back to the stay-on-post answer");
            return false;
        }
        self.enemy()
            .profile(&self.assets.profile_manager)
            .initiative
            >= 50
    }

    fn act_on_civilian_report(&mut self) {
        match self.enemy().get_rank(&self.assets.profile_manager) {
            ProfileRank::Officer => {
                if self.seek_before_alert() {
                    self.seek(
                        self.enemy().base.seek_position,
                        0,
                        SeekFlags::LOCATION_FIRST | SeekFlags::LOOK_FOR_HELP_AFTER,
                    );
                } else if !self.engine.execute_ai_alert_soldiers(
                    self.sim,
                    self.assets,
                    self.owner,
                    self.enemy().base.seek_position,
                    0,
                ) {
                    self.duty();
                }
            }
            ProfileRank::Soldier => {
                if self.seek_before_alert() {
                    self.seek(
                        self.enemy().base.seek_position,
                        parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                        SeekFlags::LOCATION_FIRST | SeekFlags::LOOK_FOR_HELP_AFTER,
                    );
                } else if !self
                    .engine
                    .execute_ai_alert_officer(self.sim, self.assets, self.owner)
                {
                    self.seek(
                        self.enemy().base.seek_position,
                        parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                        SeekFlags::LOCATION_FIRST,
                    );
                }
            }
            ProfileRank::Knight => self.seek(
                self.enemy().base.seek_position,
                parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                SeekFlags::LOCATION_FIRST,
            ),
            _ => {}
        }
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
