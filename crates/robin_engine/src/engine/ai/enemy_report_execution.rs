//! Synchronous report conversations, with participants borrowed only between calls.

use super::*;
use crate::ai::{
    AiEntityHandle, AiSpeechAttempt, AiState, DutyFlags, Remark, ReportType, SpeechFlags, Stimulus,
    StimulusInfo, Substate,
};
use crate::ai_enemy::SeekFlags;
#[cfg(test)]
use crate::sim_rng::SimulationContext;

impl EngineInner {
    #[cfg(test)]
    pub(in crate::engine) fn execute_enemy_report_callback(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        AiOwnerCtx::new(self, sim, assets, owner).execute_enemy_report_callback(stimulus)
    }

    fn report_antagonist(&self, owner: EntityId) -> EntityId {
        let handle = self
            .ai(owner, "report participant")
            .antagonist
            .expect("report conversation requires an antagonist");
        self.expect_human_id_for_ai_handle(handle.get(), "report antagonist")
    }

    fn report_timer(&mut self, owner: EntityId, frames: u32) {
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("report timer"))
            .launch_timer(frames, self.control.frame_counter);
    }

    /// Transfer a report directly from its owning brain. Only the loop extent is
    /// retained; body handles and the report metadata are read at their use sites.
    pub(in crate::engine) fn consider_live_ai_report(
        &mut self,
        recipient: EntityId,
        donor: EntityId,
        flags: u16,
    ) {
        let count = self
            .ai(donor, "report donor")
            .my_reconnaissance_report
            .seen_bodies
            .len();
        for index in 0..count {
            let body = self
                .ai(donor, "report body donor")
                .my_reconnaissance_report
                .seen_bodies[index];
            if self
                .ai(recipient, "report body recipient")
                .my_reconnaissance_report
                .is_body_seen(body)
            {
                continue;
            }
            let body_id = self.expect_human_id_for_ai_handle(body, "reported body");
            let ai = self.ai_mut(recipient, "report body merge");
            if flags & 1 != 0 {
                ai.my_reconnaissance_report.add_seen_body(body);
            }
            self.execute_ai_delete_detectable_entity(recipient, body_id, DetectableType::Body);
        }
        let charly = self
            .ai(donor, "report Charly donor")
            .my_reconnaissance_report
            .charly;
        let ai = self.ai_mut(recipient, "report Charly merge");
        if flags & 2 != 0
            && ai.my_reconnaissance_report.charly.is_none()
            && let Some(charly) = charly
        {
            ai.my_reconnaissance_report.charly = Some(charly);
            ai.my_reconnaissance_report.charly_seen = false;
            self.execute_ai_append_detectable(
                recipient,
                EntityId::Soldier(SoldierId(charly.get())),
                DetectableType::MissedFriend,
            );
        }
        if flags & 4 != 0 {
            let report = &self.ai(donor, "report type donor").my_reconnaissance_report;
            let (kind, position) = (report.report_type, report.seek_position);
            self.ai_mut(recipient, "report type merge")
                .my_reconnaissance_report
                .update(kind, position);
        }
    }
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_enemy_report_callback(
        &mut self,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        let enemy = self.engine.entities().get(self.owner)?.enemy_ai()?;
        let substate = enemy.base.current_substate;
        let kind = stimulus.stimulus_type;
        if !kind.is_expected_class() {
            return None;
        }
        match substate {
            Substate::SeekingSoldierReturnToOfficer => {
                if matches!(
                    kind,
                    StimulusType::EventTimer | StimulusType::EventReachPoint
                ) {
                    let officer = self.engine.report_antagonist(self.owner);
                    let officer_substate = self
                        .engine
                        .ai(officer, "returning soldier's officer")
                        .current_substate;
                    let waiting = matches!(
                        officer_substate,
                        Substate::SeekingOfficerWaitForInstructedSoldier
                            | Substate::SeekingOfficerWaitForInstructedGroup
                    );
                    if kind == StimulusType::EventTimer {
                        let position = self.engine.live_ai_position(self.owner);
                        let remembered = self
                            .engine
                            .enemy_ai(self.owner, "returning soldier")
                            .officers_position;
                        let dx = position.x - remembered.x;
                        let dy = position.y - remembered.y;
                        if waiting
                            || officer_substate == Substate::SeekingDetectedCharly
                            || dx * dx + dy * dy
                                >= (self.engine.ai.standard_view_polygon_radius as f32).powi(2)
                        {
                            self.engine.report_timer(self.owner, 20);
                        } else {
                            self.execute_ai_return_to_duty(DutyFlags::empty());
                        }
                    } else if waiting {
                        self.engine.execute_ai_callback(
                            self.sim,
                            self.assets,
                            officer,
                            &Stimulus::with_human(StimulusType::CallReport, self.owner.index()),
                        );
                        self.execute_ai_speech(AiSpeechAttempt {
                            remark: Remark::TellsOfficerNothing,
                            flags: SpeechFlags::MYTALK_1.bits(),
                        });
                        self.report_state(Substate::SeekingSoldierGiveReportToOfficer);
                        self.engine.report_timer(self.owner, 100);
                    } else {
                        self.execute_ai_return_to_duty(DutyFlags::empty());
                    }
                }
            }
            Substate::SeekingSoldierGiveReportToOfficer => match kind {
                StimulusType::EventMyTalk1 => {
                    let officer = self.engine.report_antagonist(self.owner);
                    self.engine.execute_ai_callback(
                        self.sim,
                        self.assets,
                        officer,
                        &Stimulus::with_human(StimulusType::CallYourTalk1, self.owner.index()),
                    );
                    self.engine.report_timer(self.owner, 20);
                }
                StimulusType::EventTimer => {
                    self.engine
                        .enemy_ai_mut(self.owner, "report completion")
                        .seek_flags = SeekFlags::empty();
                    self.execute_ai_return_to_duty(DutyFlags::empty());
                }
                _ => {}
            },
            Substate::SeekingOfficerGetReportFromSoldier => match kind {
                StimulusType::CallYourTalk1 => self.execute_ai_speech(AiSpeechAttempt {
                    remark: Remark::OfficerEndsConversation,
                    flags: SpeechFlags::MYTALK_1.bits(),
                }),
                StimulusType::EventTimer | StimulusType::EventMyTalk1 => {
                    self.execute_ai_return_to_duty(DutyFlags::empty());
                }
                _ => {}
            },
            Substate::SeekingRunningToOfficerSeen => {
                if matches!(
                    kind,
                    StimulusType::EventMyTalk0
                        | StimulusType::EventTimer
                        | StimulusType::EventReachPoint
                ) {
                    let officer = self.engine.report_antagonist(self.owner);
                    let waiting = matches!(
                        self.engine
                            .world
                            .entities
                            .expect_ai_controller(officer, format_args!("alert report officer"))
                            .current_substate,
                        Substate::SeekingOfficerWaitForInstructedSoldier
                            | Substate::SeekingOfficerWaitForAlertingSoldier
                            | Substate::SeekingDetectedCharly
                    );
                    if kind == StimulusType::EventMyTalk0 {
                        if waiting {
                            self.engine.execute_ai_callback(
                                self.sim,
                                self.assets,
                                officer,
                                &Stimulus::new(StimulusType::CallYourTalk0),
                            );
                        }
                    } else if !waiting {
                        self.execute_ai_return_to_duty(DutyFlags::empty());
                    } else if kind == StimulusType::EventTimer {
                        self.engine.report_timer(self.owner, 20);
                    } else {
                        self.report_state(Substate::SeekingSoldierGiveAlertingReportToOfficerStart);
                        let remark = match self
                            .engine
                            .ai(self.owner, "alert report speech")
                            .my_reconnaissance_report
                            .report_type
                        {
                            ReportType::Body | ReportType::DeadBody => Remark::TellsOfficerBody,
                            ReportType::Enemy => Remark::TellsOfficerEnemy,
                            ReportType::MissedCharly => Remark::TellsOfficerCharlyAway,
                            _ => Remark::TellsOfficerOther,
                        };
                        self.execute_ai_speech(AiSpeechAttempt {
                            remark,
                            flags: (SpeechFlags::MYTALK_1 | SpeechFlags::EMERGENCY).bits(),
                        });
                        self.engine.report_timer(self.owner, 150);
                    }
                }
            }
            Substate::SeekingSoldierGiveAlertingReportToOfficerStart => {
                if matches!(kind, StimulusType::EventMyTalk1 | StimulusType::EventTimer) {
                    let officer = self.engine.report_antagonist(self.owner);
                    let officer_report = self
                        .engine
                        .ai(officer, "alert report recipient")
                        .my_reconnaissance_report
                        .report_type;
                    let point = match self
                        .engine
                        .ai(self.owner, "alert report donor")
                        .my_reconnaissance_report
                        .report_type
                    {
                        ReportType::Nothing => false,
                        ReportType::Noise | ReportType::MissedCharly => {
                            officer_report == ReportType::Nothing
                        }
                        ReportType::Body | ReportType::DeadBody => {
                            officer_report <= ReportType::Noise
                        }
                        ReportType::Enemy => officer_report <= ReportType::DeadBody,
                    };
                    if point {
                        self.engine.execute_ai_callback(
                            self.sim,
                            self.assets,
                            officer,
                            &Stimulus::with_human(StimulusType::CallReport, self.owner.index()),
                        );
                        // The recipient can redirect the conversation during the report.
                        let officer = self.engine.report_antagonist(self.owner);
                        self.engine.execute_ai_callback(
                            self.sim,
                            self.assets,
                            officer,
                            &Stimulus::new(StimulusType::CallYourTalk1),
                        );
                        self.report_state(Substate::SeekingSoldierGiveAlertingReportToOfficerPoint);
                        self.engine.report_timer(self.owner, 100);
                    } else {
                        self.report_state(Substate::SeekingSoldierGiveAlertingReportToOfficerEnd);
                        let officer = self.engine.report_antagonist(self.owner);
                        self.engine.execute_ai_callback(
                            self.sim,
                            self.assets,
                            officer,
                            &Stimulus::with_human(StimulusType::CallReport, self.owner.index()),
                        );
                        self.engine.report_timer(
                            self.owner,
                            crate::parameters_ai::AI_STANDARD_TALK_TIME as u32,
                        );
                    }
                }
            }
            Substate::SeekingSoldierGiveAlertingReportToOfficerPoint => match kind {
                StimulusType::CallYourTalk1 | StimulusType::EventTimer => {
                    self.execute_ai_speech(AiSpeechAttempt {
                        remark: Remark::TellsOfficerWhere,
                        flags: SpeechFlags::empty().bits(),
                    });
                    let position = self.engine.ai(self.owner, "report pointing").seek_position;
                    self.duty_point_to(position);
                }
                StimulusType::EventDone => {
                    self.report_state(Substate::SeekingSoldierGiveAlertingReportToOfficerEnd);
                    let officer = self.engine.report_antagonist(self.owner);
                    self.report_face(officer);
                    self.engine.report_timer(
                        self.owner,
                        crate::parameters_ai::AI_STANDARD_TALK_TIME as u32,
                    );
                }
                _ => {}
            },
            Substate::SeekingSoldierGiveAlertingReportToOfficerEnd => {
                if kind == StimulusType::EventTimer {
                    self.engine
                        .enemy_ai_mut(self.owner, "alert report completion")
                        .seek_flags = SeekFlags::empty();
                    self.execute_ai_return_to_duty(DutyFlags::empty());
                }
            }
            Substate::SeekingOfficerGetAlertingReportFromSoldier
                if matches!(
                    kind,
                    StimulusType::CallYourTalk1 | StimulusType::EventMyTalk1
                ) =>
            {
                if kind == StimulusType::CallYourTalk1 {
                    self.execute_ai_speech(AiSpeechAttempt {
                        remark: Remark::OfficerAsksWhere,
                        flags: (SpeechFlags::MYTALK_1 | SpeechFlags::EMERGENCY).bits(),
                    });
                } else {
                    let soldier = self.engine.report_antagonist(self.owner);
                    self.engine.execute_ai_callback(
                        self.sim,
                        self.assets,
                        soldier,
                        &Stimulus::new(StimulusType::CallYourTalk1),
                    );
                }
            }
            Substate::SeekingOfficerWaitForInstructedSoldier
            | Substate::SeekingOfficerWaitForInstructedGroup
            | Substate::SeekingOfficerWaitForAlertingSoldier
            | Substate::SeekingWaitForAlertingCivilian
                if kind == StimulusType::CallReport =>
            {
                let civilian = substate == Substate::SeekingWaitForAlertingCivilian;
                let sender = match &stimulus.info {
                    StimulusInfo::Human(sender) => *sender,
                    _ => panic!("report requires a reporting human"),
                };
                let donor = self
                    .engine
                    .expect_human_id_for_ai_handle(sender.get(), "reporting human");
                assert!(
                    matches!(
                        (civilian, self.engine.world.entities.get(donor)),
                        (true, Some(Entity::Civilian(_))) | (false, Some(Entity::Soldier(_)))
                    ),
                    "report donor has the wrong actor kind"
                );
                let old_type = self
                    .engine
                    .ai(self.owner, "report recipient")
                    .my_reconnaissance_report
                    .report_type;
                self.engine.consider_live_ai_report(self.owner, donor, 7);
                if !civilian {
                    self.engine.consider_live_ai_report(donor, self.owner, 0);
                }
                let report = &self
                    .engine
                    .ai(donor, "report donor after handoff")
                    .my_reconnaissance_report;
                let report_type = report.report_type;
                let group = substate == Substate::SeekingOfficerWaitForInstructedGroup;
                let alerting = report_type > old_type
                    && if civilian {
                        report_type >= ReportType::Body
                    } else {
                        report_type > ReportType::Body
                    }
                    && (!group || old_type == ReportType::MissedCharly);
                if alerting {
                    self.report_state(if civilian {
                        Substate::SeekingGetAlertingReportFromCivilian
                    } else {
                        Substate::SeekingOfficerGetAlertingReportFromSoldier
                    });
                    self.engine.ai_mut(self.owner, "alert report").antagonist =
                        Some(AiEntityHandle::new(donor.index()));
                    self.report_face(donor);
                    let report = &self
                        .engine
                        .ai(donor, "alert report after facing")
                        .my_reconnaissance_report;
                    let (report_type, seek_position) = (report.report_type, report.seek_position);
                    let ai = self.engine.ai_mut(self.owner, "alert report position");
                    ai.seek_position = seek_position;
                    ai.my_reconnaissance_report
                        .update(report_type, seek_position);
                } else if group {
                    self.report_face(donor);
                } else {
                    self.report_state(if civilian {
                        Substate::SeekingGetReportFromCivilian
                    } else {
                        Substate::SeekingOfficerGetReportFromSoldier
                    });
                    let antagonist = self.engine.report_antagonist(self.owner);
                    self.report_face(antagonist);
                }
                let frames =
                    if !alerting && substate == Substate::SeekingOfficerWaitForInstructedSoldier {
                        100
                    } else {
                        crate::parameters_ai::AI_STANDARD_TALK_TIME as u32
                    };
                self.engine.report_timer(self.owner, frames);
            }
            _ => return None,
        }
        Some(false)
    }

    fn report_state(&mut self, substate: Substate) {
        self.duty_set_state(AiState::Seeking, substate);
    }

    fn report_face(&mut self, target: EntityId) {
        let position = self.engine.live_ai_position(target);
        let elevation = self
            .engine
            .expect_entity(target, "report facing target")
            .position_iface()
            .get_elevation() as i16 as f32;
        self.duty_face_position_at_elevation(position, elevation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::test_support::actors::{
        make_test_ai_soldier, make_test_civilian, make_test_pc,
    };

    #[test]
    fn live_report_deletes_pc_body_detection_without_copying_body_history() {
        let mut engine = EngineInner::new();
        let recipient = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
        let donor = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
        let pc = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
        engine
            .world
            .entities
            .expect_ai_controller_mut(donor, format_args!("body report donor"))
            .my_reconnaissance_report
            .add_seen_body(pc.index());
        engine
            .world
            .entities
            .get_mut(recipient)
            .unwrap()
            .npc_data_mut()
            .unwrap()
            .detectable_lists[DetectableType::Body as usize]
            .push(crate::element::Detectable {
                element: Some(pc),
                detectable_type: DetectableType::Body,
                ..Default::default()
            });
        engine.consider_live_ai_report(recipient, donor, 0);
        let recipient = engine.world.entities.get(recipient).unwrap();
        assert!(
            recipient.npc_data().unwrap().detectable_lists[DetectableType::Body as usize]
                .is_empty()
        );
        assert!(
            recipient
                .ai_controller()
                .unwrap()
                .my_reconnaissance_report
                .seen_bodies
                .is_empty()
        );
    }

    #[test]
    #[should_panic(expected = "reporting human")]
    fn civilian_report_does_not_fabricate_enemy_data_when_sender_is_missing() {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
        let ai = engine
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("test listener"));
        ai.current_state = AiState::Seeking;
        ai.current_substate = Substate::SeekingWaitForAlertingCivilian;
        engine.execute_enemy_report_callback(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            owner,
            &Stimulus::with_human(StimulusType::CallReport, 42),
        );
    }

    #[test]
    fn transferred_charly_starts_unseen_and_appends_existing_detection() {
        let mut engine = EngineInner::new();
        let recipient = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
        let donor = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
        let charly = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
        engine
            .world
            .entities
            .get_mut(recipient)
            .unwrap()
            .npc_data_mut()
            .unwrap()
            .detectable_lists[DetectableType::MissedFriend as usize]
            .push(crate::element::Detectable {
                element: Some(charly),
                detectable_type: DetectableType::MissedFriend,
                ..Default::default()
            });
        engine
            .world
            .entities
            .expect_ai_controller_mut(recipient, format_args!("report recipient fixture"))
            .my_reconnaissance_report
            .charly_seen = true;
        engine
            .world
            .entities
            .expect_ai_controller_mut(donor, format_args!("report donor fixture"))
            .my_reconnaissance_report
            .charly = Some(AiEntityHandle::new(charly.index()));
        engine.consider_live_ai_report(recipient, donor, 2);
        let report = &engine
            .world
            .entities
            .expect_ai_controller(recipient, format_args!("transferred report"))
            .my_reconnaissance_report;
        assert_eq!(report.charly, Some(AiEntityHandle::new(charly.index())));
        assert!(!report.charly_seen);
        let list = &engine
            .world
            .entities
            .get(recipient)
            .unwrap()
            .npc_data()
            .unwrap()
            .detectable_lists[DetectableType::MissedFriend as usize];
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|entry| entry.element == Some(charly)));
    }

    #[test]
    fn report_substate_dispatch_does_not_depend_on_state_field() {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
        let ai = engine
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("report substate fixture"));
        ai.current_state = AiState::Default;
        ai.current_substate = Substate::SeekingSoldierGiveReportToOfficer;
        assert_eq!(
            engine.execute_enemy_report_callback(
                &crate::sim_rng::test_context(),
                &LevelAssets::new(),
                owner,
                &Stimulus::new(StimulusType::EventDone),
            ),
            Some(false)
        );
    }

    #[test]
    fn returning_soldier_with_far_civilian_antagonist_keeps_route_and_rearms_timer() {
        let sim = crate::sim_rng::test_context();
        let mut engine = EngineInner::new();
        engine.control.frame_counter = 14_748;
        engine.add_test_entity(make_test_civilian(crate::element::Posture::Upright));
        let civilian = engine.add_test_entity(make_test_civilian(crate::element::Posture::Upright));
        let owner = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
        let Entity::Civilian(actor) = engine.get_entity_mut(civilian).unwrap() else {
            unreachable!()
        };
        let mut friendly = crate::ai_friendly::FriendlyAi::new(civilian.index());
        friendly.base.current_state = AiState::Fleeing;
        friendly.base.current_substate = Substate::FleeingHiding;
        actor.npc.ai_brain = crate::element::AiBrain::Friendly(Box::new(friendly));
        engine
            .get_entity_mut(owner)
            .unwrap()
            .element_data_mut()
            .set_position_map(MapPoint::new(2_006.434_9, 1_735.375_2));
        let ai = engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("test reporter"));
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingSoldierReturnToOfficer;
        ai.base.antagonist = Some(AiEntityHandle::new(civilian.index()));
        ai.officers_position = crate::ai::Position {
            x: 1_503.635_1,
            y: 1_097.013_8,
            ..Default::default()
        };
        engine.ai.standard_view_polygon_radius = 300;
        assert_eq!(
            engine.execute_enemy_report_callback(
                &sim,
                &LevelAssets::new(),
                owner,
                &Stimulus::new(StimulusType::EventTimer),
            ),
            Some(false)
        );
        let ai = engine
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("test reporter result"));
        assert_eq!(ai.base.current_state, AiState::Seeking);
        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingSoldierReturnToOfficer
        );
        assert!(ai.base.timer_is_running);
        assert_eq!(ai.base.when_does_timer_ring, 14_768);
        assert_eq!(
            engine.execute_enemy_report_callback(
                &sim,
                &LevelAssets::new(),
                owner,
                &Stimulus::new(StimulusType::EventLoseConsciousness),
            ),
            None
        );
    }
}
