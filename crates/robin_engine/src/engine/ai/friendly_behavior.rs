use super::*;
use crate::ai::{
    AiEntityHandle, AiState, DutyFlags, GotoFlags, Position, Remark, ReportType, SpeechFlags,
    Stimulus, StimulusInfo, StimulusType, Substate,
};
use crate::sim_rng::SimulationContext;

impl EngineInner {
    pub(in crate::engine) fn execute_friendly_remaining_event(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> bool {
        use StimulusType::*;
        match stimulus.stimulus_type {
            EventReachPoint | EventDone | EventTimer | CallYourTalk1 | CallYourTalk2
            | CallYourTalk3 | EventMyTalk1 | EventMyTalk2 | EventMyTalk3 => {
                if let Some(result) =
                    self.execute_ai_common_fleeing_event(sim, assets, owner, stimulus)
                {
                    return result;
                }
                if let Some(result) =
                    self.execute_ai_common_expected_event(sim, assets, owner, stimulus)
                {
                    return result;
                }
            }
            EventCouldntReachPoint => {
                if self.friendly_brain(owner).base.current_substate == Substate::FleeingPanic {
                    self.execute_ai_common_fleeing_event(sim, assets, owner, stimulus)
                        .expect("panic failure requires common fleeing handler");
                } else {
                    self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
                }
            }
            EventFitAgain => {
                self.broadcast_resurrection(owner);
                let actor = self
                    .world
                    .entities
                    .expect_ai_actor_data_mut(owner, format_args!("civilian recovery eyes"));
                crate::ai_vision::set_view_status(actor, crate::element::EyeStatus::LookForward);
                self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
            }
            EventReturnToDuty => {
                self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty())
            }
            EventOutOfView | EventSeesShadow | EventSeesSoldier => {}
            _ => {
                tracing::warn!(event = ?stimulus.stimulus_type, "unhandled civilian stimulus");
            }
        }
        false
    }

    pub(in crate::engine) fn begin_friendly_think(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> bool {
        let frame = self.control.frame_counter;
        let ai = self.reporting_civilian_mut(owner);
        ai.base.cached_frame = frame;
        if !self.admit_ai_think_live(owner, stimulus) {
            return false;
        }
        let (state, substate, eye_status, refused) = match stimulus.stimulus_type {
            StimulusType::EventLoseConsciousness => {
                self.execute_ai_break_macro(owner);
                self.reporting_civilian_mut(owner).base.clear_emoticon();
                (
                    AiState::Sleeping,
                    Substate::SleepingUnconscious,
                    crate::element::EyeStatus::DieOrGetUnconscious,
                    13,
                )
            }
            StimulusType::EventWasp => {
                self.execute_ai_break_macro(owner);
                self.reporting_civilian_mut(owner)
                    .base
                    .set_emoticon(crate::ai::EmoticonType::Thunderstorm);
                (
                    AiState::Wondering,
                    Substate::WonderingWaspInArmour,
                    crate::element::EyeStatus::Closed,
                    14,
                )
            }
            StimulusType::EventNet => {
                self.execute_ai_break_macro(owner);
                (
                    AiState::Wondering,
                    Substate::WonderingUnderNet,
                    crate::element::EyeStatus::Closed,
                    15,
                )
            }
            _ => return true,
        };
        self.duty_set_state(sim, assets, owner, state, substate);
        let actor = self
            .world
            .entities
            .expect_ai_actor_data_mut(owner, format_args!("civilian admission eye status"));
        crate::ai_vision::set_view_status(actor, eye_status);
        if stimulus.stimulus_type == StimulusType::EventLoseConsciousness {
            self.execute_ai_set_alert_status(
                assets,
                owner,
                crate::ai::AlertLevel::Green,
                crate::ai::AlertFlags::empty(),
            );
        }
        let ai = self.reporting_civilian_mut(owner);
        ai.base.sorrow_level = 0;
        ai.base
            .register_log_line(crate::ai::LogLineType::EventRefused, refused);
        false
    }

    pub(super) fn execute_friendly_behavior(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        use StimulusType::*;
        let event = stimulus.stimulus_type;
        let state = self.friendly_brain(owner).base.current_state;
        match event {
            EventView => {
                let StimulusInfo::Human(handle) = stimulus.info else {
                    panic!("civilian view needs human");
                };
                let target =
                    self.expect_human_id_for_ai_handle(handle.get(), "civilian sight target");
                match state {
                    AiState::Default | AiState::Wondering => {
                        self.execute_civilian_view(sim, assets, owner, target)
                    }
                    AiState::Seeking => {
                        if self.expect_entity(owner, "civilian reporter").camp()
                            != self.expect_entity(target, "civilian sight target").camp()
                        {
                            let position = self.live_ai_position(target);
                            self.reporting_civilian_mut(owner)
                                .base
                                .my_reconnaissance_report
                                .update(ReportType::Enemy, position);
                        }
                    }
                    AiState::Fleeing => {
                        let actor = self.expect_entity(target, "civilian feared human");
                        let fear = actor.camp()
                            != self.expect_entity(owner, "civilian observer").camp()
                            || !actor
                                .human_data()
                                .expect("view target human")
                                .opponents
                                .is_empty();
                        let ai = self.friendly_brain(owner);
                        if fear
                            && (ai.base.current_substate == Substate::FleeingHiding
                                || ai.fleeing_seen_enemy_counter < 7)
                        {
                            self.reporting_civilian_mut(owner)
                                .fleeing_seen_enemy_counter += 1;
                            self.execute_ai_speech(
                                sim,
                                assets,
                                owner,
                                crate::ai::AiSpeechAttempt {
                                    remark: Remark::CivPanic,
                                    flags: SpeechFlags::HOUSE.bits(),
                                },
                            );

                            self.civilian_panic_from_human(sim, assets, owner, target);
                        }
                    }
                    _ => panic!("civilian view in invalid state {state:?}"),
                }
            }
            EventSeesBody => {
                if matches!(state, AiState::Default | AiState::Wondering) {
                    let StimulusInfo::Human(handle) = stimulus.info else {
                        panic!("civilian body sight needs human");
                    };
                    let target =
                        self.expect_human_id_for_ai_handle(handle.get(), "civilian body target");
                    self.execute_civilian_body_view(sim, assets, owner, target);
                }
            }
            EventPanic => {
                let StimulusInfo::Position(position) = stimulus.info else {
                    panic!("civilian panic needs position");
                };
                self.execute_ai_panic(
                    sim,
                    assets,
                    owner,
                    Some(position),
                    crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
                    crate::ai::AlertLevel::Red,
                );
            }
            EventStop => {
                if state != AiState::Sleeping {
                    self.civilian_stop(sim, assets, owner);
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Seeking,
                        Substate::SeekingGotStopEvent,
                    );
                    self.civilian_timer(owner, 100);
                }
            }
            CallYouJustWait | EventAppleChaseNear => {
                let StimulusInfo::Human(chaser) = stimulus.info else {
                    panic!("child chase requires soldier");
                };
                self.reporting_civilian_mut(owner).base.antagonist = Some(chaser);
                if let Some(destination) = self.live_child_flee_destination(sim, owner) {
                    let substate = if event == CallYouJustWait {
                        Substate::FleeingChildChased
                    } else {
                        Substate::FleeingChildFriendChased
                    };
                    self.duty_set_state(sim, assets, owner, AiState::Fleeing, substate);
                    self.duty_go_to(sim, assets, owner, destination, GotoFlags::RUN);
                } else {
                    let target = self.civilian_chaser(owner);
                    self.civilian_panic_from_human(sim, assets, owner, target);
                }
            }
            EventNetAway => {
                let position = self.friendly_brain(owner).base.seek_position;
                self.execute_ai_panic(
                    sim,
                    assets,
                    owner,
                    Some(position),
                    crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
                    crate::ai::AlertLevel::Red,
                );
            }
            EventPcShotAtMe
            | EventSeesObject
            | EventSeesFriendInTrouble
            | EventGotHit
            | EventLoseConsciousness
            | EventGetArrow => {}
            EventReachPoint | EventDone | EventTimer | CallYourTalk1 | CallYourTalk2
            | CallYourTalk3 | EventMyTalk1 | EventMyTalk2 | EventMyTalk3 => {
                return self.execute_civilian_expected_behavior(sim, assets, owner, event);
            }
            _ => return None,
        }
        Some(false)
    }

    fn friendly_brain(&self, owner: EntityId) -> &crate::ai_friendly::FriendlyAi {
        self.expect_entity(owner, "civilian behavior owner")
            .friendly_ai()
            .expect("civilian behavior requires FriendlyAi")
    }

    fn civilian_timer(&mut self, owner: EntityId, duration: u32) {
        let frame = self.control.frame_counter;
        self.reporting_civilian_mut(owner)
            .base
            .launch_timer(duration, frame);
    }

    fn civilian_stop(&mut self, sim: &SimulationContext, assets: &LevelAssets, owner: EntityId) {
        self.stop_ai_owner(sim, assets, owner);
    }

    fn civilian_chaser(&self, owner: EntityId) -> EntityId {
        self.expect_human_id_for_ai_handle(
            self.friendly_brain(owner)
                .base
                .antagonist
                .expect("child needs chaser")
                .get(),
            "child chaser",
        )
    }

    fn civilian_face_position(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        position: Position,
    ) {
        let target = crate::ai::ai_position_to_point_3d(
            &self.world.fast_grid,
            self.sight_obstacles(assets),
            position,
        );
        let body = self
            .expect_entity(owner, "civilian facing owner")
            .element_data()
            .position();
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
            target.x - body.x,
            target.y - body.y,
        );
        self.duty_face_direction(sim, assets, owner, direction as u16);
    }

    fn civilian_face_human(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: EntityId,
    ) {
        let position = self.live_ai_position(target);
        let elevation = self
            .expect_entity(target, "civilian facing human")
            .position_iface()
            .get_elevation() as i16;
        self.duty_face_position_at_elevation(sim, assets, owner, position, f32::from(elevation));
    }

    fn civilian_panic_from_human(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: EntityId,
    ) {
        let position = self.live_ai_position(target);
        self.execute_ai_panic(
            sim,
            assets,
            owner,
            Some(position),
            crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
            crate::ai::AlertLevel::Red,
        );
    }

    fn execute_civilian_view(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: EntityId,
    ) {
        let entity = self.expect_entity(target, "civilian viewed human");
        if !entity
            .human_data()
            .expect("viewed human")
            .opponents
            .is_empty()
        {
            self.civilian_panic_from_human(sim, assets, owner, target);
        } else if entity.camp() == self.expect_entity(owner, "civilian viewer").camp() {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingCivilianAdmiringHero,
            );
            if matches!(self.expect_entity(target, "civilian admired human"), Entity::Pc(pc) if pc.is_robin())
            {
                self.execute_ai_speech(
                    sim,
                    assets,
                    owner,
                    crate::ai::AiSpeechAttempt {
                        remark: Remark::CivAdmiresRobin,
                        flags: 0,
                    },
                );
            }
            self.civilian_stop(sim, assets, owner);
            self.civilian_face_human(sim, assets, owner, target);
            self.civilian_timer(owner, crate::parameters_ai::AI_FIRST_LOOK_TIME as u32);
        } else if self.entity_data_in_building_sector(
            self.expect_entity(owner, "civilian viewer").element_data(),
        ) {
            self.execute_ai_speech(
                sim,
                assets,
                owner,
                crate::ai::AiSpeechAttempt {
                    remark: Remark::CivPanic,
                    flags: SpeechFlags::HOUSE.bits(),
                },
            );

            self.execute_ai_panic(
                sim,
                assets,
                owner,
                None,
                crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
                crate::ai::AlertLevel::Red,
            );
        } else {
            let position = self.live_ai_position(target);
            let ai = self.reporting_civilian_mut(owner);
            ai.base.primary_target = Some(AiEntityHandle::new(target.index()));
            ai.base.seek_position = position;
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingCivilianEnemyReactiontime,
            );
            self.civilian_stop(sim, assets, owner);
            let position = self.friendly_brain(owner).base.seek_position;
            self.reporting_civilian_mut(owner)
                .base
                .my_reconnaissance_report
                .update(ReportType::Enemy, position);
            self.civilian_face_position(sim, assets, owner, position);
            self.civilian_timer(owner, 30);
        }
    }

    fn execute_civilian_body_view(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: EntityId,
    ) {
        let position = self.live_ai_position(target);
        self.reporting_civilian_mut(owner).base.seek_position = position;
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Wondering,
            Substate::WonderingCivilianBodyReactiontime,
        );
        self.civilian_stop(sim, assets, owner);
        self.execute_ai_speech(
            sim,
            assets,
            owner,
            crate::ai::AiSpeechAttempt {
                remark: Remark::CivSeesBody,
                flags: 0,
            },
        );

        let position = self.live_ai_position(target);
        self.reporting_civilian_mut(owner)
            .base
            .my_reconnaissance_report
            .update(ReportType::Body, position);
        let position = self.friendly_brain(owner).base.seek_position;
        self.civilian_face_position(sim, assets, owner, position);
        self.civilian_timer(owner, crate::parameters_ai::AI_FIRST_LOOK_TIME as u32);
    }

    fn execute_civilian_expected_behavior(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        event: StimulusType,
    ) -> Option<bool> {
        use StimulusType::*;
        let substate = self.friendly_brain(owner).base.current_substate;
        match substate {
            Substate::DefaultPatrolEnroute | Substate::DefaultPatrolEnrouteRunning => {
                if event == EventReachPoint {
                    let direction = self.friendly_brain(owner).base.patrol_direction;
                    if direction
                        != self
                            .expect_entity(owner, "patrol arrival")
                            .element_data()
                            .direction() as u16
                    {
                        self.duty_face_direction(sim, assets, owner, direction);
                    }
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Default,
                        Substate::DefaultPatrolEnrouteWaiting,
                    );
                }
            }
            Substate::DefaultChildApproachedWhistling
            | Substate::WonderingCivilianAdmiringHero
            | Substate::SeekingGotStopEvent
            | Substate::FleeingChildChasedEnd => {
                if event == EventTimer {
                    self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
                }
            }
            Substate::WonderingWatchingWhistling => {
                if event == EventTimer {
                    self.execute_ai_speech(
                        sim,
                        assets,
                        owner,
                        crate::ai::AiSpeechAttempt {
                            remark: Remark::CivWhistling,
                            flags: 0,
                        },
                    );

                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Wondering,
                        Substate::WonderingChildApproachingWhistling,
                    );
                    let position = self.friendly_brain(owner).base.seek_position;
                    self.duty_go_near(sim, assets, owner, position, 50, GotoFlags::RUN);
                }
            }
            Substate::WonderingChildApproachingWhistling => {
                if event == EventReachPoint {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Default,
                        Substate::DefaultChildApproachedWhistling,
                    );
                    self.civilian_timer(owner, 100);
                }
            }
            Substate::FleeingChildChased => match event {
                CallYourTalk1 => self.execute_ai_speech(
                    sim,
                    assets,
                    owner,
                    crate::ai::AiSpeechAttempt {
                        remark: Remark::CivChildChasedBySoldier,
                        flags: 0,
                    },
                ),
                EventReachPoint => {
                    if let Some(goal) = self.live_child_flee_destination(sim, owner) {
                        let target = self.civilian_chaser(owner);
                        let here = self
                            .expect_entity(owner, "fleeing child")
                            .element_data()
                            .position();
                        let there = self
                            .expect_entity(target, "child chaser")
                            .element_data()
                            .position();
                        let distance = (here.x - there.x).abs().max(
                            ((here.y - there.y) * crate::position_interface::INVERSE_ASPECT_RATIO)
                                .abs(),
                        );
                        self.duty_go_to_speed(
                            sim,
                            assets,
                            owner,
                            goal,
                            GotoFlags::RUN | GotoFlags::DONT_STOP,
                            if distance < 150.0 { 1.2 } else { 1.0 },
                        );
                        let target = self.civilian_chaser(owner);
                        let substate = self
                            .expect_entity(target, "child chaser after movement")
                            .ai_controller()
                            .expect("chaser needs AI")
                            .current_substate;
                        if !matches!(
                            substate,
                            Substate::WonderingAppleChasingChild
                                | Substate::WonderingAppleChasingChildWaiting
                                | Substate::WonderingAppleChasingChildEnd
                        ) {
                            self.reporting_civilian_mut(owner).base.lasting_panic_runs = 1;
                            self.duty_set_state(
                                sim,
                                assets,
                                owner,
                                AiState::Fleeing,
                                Substate::FleeingChildChasedSupplementalRuns,
                            );
                        }
                    } else {
                        let target = self.civilian_chaser(owner);
                        self.civilian_panic_from_human(sim, assets, owner, target);
                    }
                }
                _ => {}
            },
            Substate::FleeingChildChasedSupplementalRuns => {
                if event == EventReachPoint {
                    let mut moved = false;
                    if self.friendly_brain(owner).base.lasting_panic_runs > 0 {
                        self.reporting_civilian_mut(owner).base.lasting_panic_runs -= 1;
                        if let Some(goal) = self.live_child_flee_destination(sim, owner) {
                            let flags = if self.friendly_brain(owner).base.lasting_panic_runs > 0 {
                                GotoFlags::RUN | GotoFlags::DONT_STOP
                            } else {
                                GotoFlags::RUN
                            };
                            self.duty_go_to(sim, assets, owner, goal, flags);
                            moved = true;
                        }
                    }
                    if !moved {
                        self.duty_set_state(
                            sim,
                            assets,
                            owner,
                            AiState::Fleeing,
                            Substate::FleeingChildChasedEnd,
                        );
                        let target = self.civilian_chaser(owner);
                        self.civilian_face_human(sim, assets, owner, target);
                        self.civilian_timer(owner, 20);
                    }
                }
            }
            Substate::FleeingChildFriendChased => {
                if event == EventReachPoint {
                    let target = self.civilian_chaser(owner);
                    let position = self.live_ai_position(target);
                    self.civilian_face_position(sim, assets, owner, position);
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Fleeing,
                        Substate::FleeingChildChasedEnd,
                    );
                    self.civilian_timer(owner, 50);
                }
            }
            _ => return None,
        }
        Some(false)
    }

    fn live_child_flee_destination(
        &self,
        sim: &SimulationContext,
        owner: EntityId,
    ) -> Option<Position> {
        let target = self.civilian_chaser(owner);
        let here = self.live_ai_position(owner);
        let there = self.live_ai_position(target);
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
            here.x - there.x,
            here.y - there.y,
        ) as i32;
        let jitter =
            crate::sim_rng::u32(sim, crate::sim_rng::RngSite::CivilianPanicDirection, 0..5) as i32;
        let direction = (direction + jitter + 14).rem_euclid(16);
        for distance in (20..=crate::ai_friendly::APPLE_CHASE_IDEAL_DISTANCE)
            .rev()
            .step_by(10)
        {
            for relative in [0, 1, -1, 2, -2, 3, -3, 4, -4, 5, -5, 6, -6, 7, -7] {
                let [x, y] = crate::position_interface::sector_to_vector_iso(
                    (direction + relative).rem_euclid(15) as i16,
                );
                let goal = Position {
                    x: here.x + x * distance as f32,
                    y: here.y + y * distance as f32,
                    ..here
                };
                if self.world.fast_grid.is_straight_movement_authorized(
                    here.map_point(),
                    goal.map_point(),
                    here.level,
                    self.expect_entity(owner, "child flee geometry")
                        .position_iface()
                        .get_move_box(),
                ) {
                    return Some(goal);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_civilian_admission_distinguishes_global_freeze_from_actor_lock() {
        for global_freeze in [false, true] {
            let (mut engine, assets, owner, _) = fixture();
            engine.ai.global.freeze = global_freeze;
            if !global_freeze {
                engine.reporting_civilian_mut(owner).base.locks_flag_field =
                    crate::ai::AiLockFlags::FREEZE;
            }

            assert!(!engine.begin_friendly_think(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                &Stimulus::new(StimulusType::EventTimer),
            ));
            let queue = &engine.friendly_brain(owner).base.stimulus_queue;
            assert_eq!(queue.len(), usize::from(!global_freeze));
            if !global_freeze {
                assert_eq!(queue[0].stimulus_type, StimulusType::EventTimer);
            }
        }
    }

    #[test]
    fn civilian_admission_special_events_complete_state_and_eye_effects_inline() {
        use crate::element::EyeStatus;
        for (event, substate, eye, refused) in [
            (
                StimulusType::EventLoseConsciousness,
                Substate::SleepingUnconscious,
                EyeStatus::DieOrGetUnconscious,
                13,
            ),
            (
                StimulusType::EventWasp,
                Substate::WonderingWaspInArmour,
                EyeStatus::Closed,
                14,
            ),
            (
                StimulusType::EventNet,
                Substate::WonderingUnderNet,
                EyeStatus::Closed,
                15,
            ),
        ] {
            let (mut engine, assets, owner, _) = fixture();
            let ai = engine.reporting_civilian_mut(owner);
            ai.base.macro_in_progress = true;
            ai.base.sorrow_level = 7;

            assert!(!engine.begin_friendly_think(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                &Stimulus::new(event),
            ));
            let ai = engine.friendly_brain(owner);
            assert_eq!(ai.base.current_substate, substate);
            assert!(!ai.base.macro_in_progress);
            assert_eq!(ai.base.sorrow_level, 0);
            assert!(ai.base.ai_log.iter().any(|line| line.line_type
                == crate::ai::LogLineType::EventRefused
                && line.info == refused));
            assert_eq!(engine.npc(owner).eye_status, eye);
        }
    }

    #[test]
    fn live_civilian_state_changes_assign_the_role_alert_levels() {
        let (mut engine, assets, owner, _) = fixture();
        for (state, substate, alert) in [
            (
                AiState::Default,
                Substate::DefaultOnPost,
                crate::ai::AlertLevel::Green,
            ),
            (
                AiState::Wondering,
                Substate::WonderingCivilianAdmiringHero,
                crate::ai::AlertLevel::Green,
            ),
            (
                AiState::Seeking,
                Substate::SeekingCivilianRunningToSoldier,
                crate::ai::AlertLevel::Yellow,
            ),
            (
                AiState::Fleeing,
                Substate::FleeingPanic,
                crate::ai::AlertLevel::Yellow,
            ),
        ] {
            engine.duty_set_state(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                state,
                substate,
            );
            let ai = engine.friendly_brain(owner);
            assert_eq!(ai.base.current_state, state);
            assert_eq!(ai.base.current_substate, substate);
            assert_eq!(ai.base.current_music_alert_status, alert);
        }
    }

    #[test]
    fn detectable_mutations_settle_before_live_state_change_and_roundtrip() {
        use crate::element::DetectableType::Friend;
        let (mut engine, assets, owner, target) = fixture();
        engine.execute_ai_append_detectable(owner, target, Friend);
        engine.execute_ai_delete_detectable_type(owner, Friend);
        let sim = crate::sim_rng::test_context();
        engine.duty_set_state(
            &sim,
            &assets,
            owner,
            AiState::Default,
            Substate::DefaultOnPost,
        );
        assert!(engine.npc(owner).detectable_lists[Friend as usize].is_empty());
        engine.execute_ai_append_detectable(owner, target, Friend);
        assert!(
            engine.npc(owner).detectable_lists[Friend as usize]
                .iter()
                .any(|entry| entry.element == Some(target))
        );
        let ai = engine.friendly_brain(owner);
        let restored: crate::ai_friendly::FriendlyAi =
            serde_json::from_str(&serde_json::to_string(ai).unwrap()).unwrap();
        assert_eq!(
            robin_util::state_hash::compute(&restored),
            robin_util::state_hash::compute(ai)
        );
    }

    fn fixture() -> (EngineInner, LevelAssets, EntityId, EntityId) {
        let (mut engine, mut assets, soldier, _) =
            super::super::battle_decision_observation_tests::fixture(false);
        let mut civilian = crate::engine::test_support::actors::make_test_civilian(
            crate::element::Posture::Leisure,
        );
        civilian
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(500.0, 500.0, 0.0));
        civilian
            .element_data_mut()
            .set_sector(engine.sector_of(soldier));
        civilian.npc_data_mut().unwrap().life_points = 50;
        civilian.npc_data_mut().unwrap().ai_brain =
            crate::element::AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(0)));
        civilian
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                crate::coordinates::MapVec::new(-10.0, -5.0),
                crate::coordinates::MapVec::new(10.0, 5.0),
            ));
        let owner = engine.add_test_entity(civilian);
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        std::sync::Arc::make_mut(&mut assets.profile_manager)
            .civilians
            .push(crate::profiles::CivilianProfile::default());
        let position = engine.live_ai_position(owner);
        let ai = engine.reporting_civilian_mut(owner);
        ai.base.initial_position = position;
        ai.base.special_action = true;
        (engine, assets, owner, soldier)
    }

    #[test]
    fn civilian_facing_a_human_uses_its_selected_door_destination() {
        let (mut engine, assets, owner, target) = fixture();
        let sector = engine.live_ai_position(owner).sector.unwrap();
        let door_index = engine.script_domains.interactables.doors.len();
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                point_in: crate::coordinates::MapPoint::new(600.0, 500.0),
                sector_in: crate::sector::SectorNumber::new(sector.get() as i16),
                sector_in_index: sector.arena_index(),
                ..Default::default()
            });
        engine.place(
            target,
            crate::coordinates::WorldPoint3D::new(600.0, 600.0, 0.0),
        );
        let mut pass = crate::sequence::SequenceElement::new_movement(
            1,
            crate::element::Command::PassDoor,
            Some(owner),
            crate::order::OrderType::WalkingUpright,
        );
        let crate::sequence::SequenceElementData::Movement {
            gate_id, direction, ..
        } = &mut pass.data
        else {
            unreachable!();
        };
        *gate_id = Some(crate::gate::DoorIndex::new(door_index as u32).unwrap());
        *direction = 1;
        let mut door_sequence = crate::sequence::Sequence::new();
        door_sequence.append_element(pass);
        let sequence = engine
            .orders
            .sequence_manager
            .insert_sequence(door_sequence);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence);
        let sim = crate::sim_rng::test_context();
        engine.select_sequence_element(owner, Some((sequence, 0)));
        engine.element_in_progress(&sim, &assets, &mut Vec::new(), sequence, 0);

        engine.civilian_face_human(&sim, &assets, owner, target);

        let turn = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| &sequence.elements)
            .find(|element| {
                element.owner == Some(owner) && element.command == crate::element::Command::Turn
            })
            .expect("human facing must register a turn while the door passage completes");
        assert!(matches!(
            turn.get_property(crate::sequence::Field::Direction),
            Some(crate::sequence::FieldValue::Integer(8))
        ));
    }

    #[test]
    fn civilian_stop_is_synchronous_and_sleeping_civilians_ignore_it() {
        for asleep in [false, true] {
            let (mut engine, assets, owner, _) = fixture();
            if asleep {
                let ai = engine.reporting_civilian_mut(owner);
                ai.base.current_state = AiState::Sleeping;
                ai.base.current_substate = Substate::SleepingForever;
            }
            assert_eq!(
                engine.execute_friendly_behavior(
                    &crate::sim_rng::test_context(),
                    &assets,
                    owner,
                    &Stimulus::new(StimulusType::EventStop)
                ),
                Some(false)
            );
            let ai = engine.friendly_brain(owner);
            assert_eq!(
                ai.base.current_substate,
                if asleep {
                    Substate::SleepingForever
                } else {
                    Substate::SeekingGotStopEvent
                }
            );
            if !asleep {
                assert_eq!(ai.base.when_does_timer_ring, 200);
            }
        }
    }

    #[test]
    fn body_sighting_reads_live_body_and_enters_reporting_reaction() {
        let (mut engine, assets, owner, body) = fixture();
        let position = engine.live_ai_position(body);
        assert_eq!(
            engine.execute_friendly_behavior(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                &Stimulus::with_human(StimulusType::EventSeesBody, body.index())
            ),
            Some(false)
        );
        let ai = engine.friendly_brain(owner);
        assert_eq!(
            ai.base.current_substate,
            Substate::WonderingCivilianBodyReactiontime
        );
        assert_eq!(
            ai.base.my_reconnaissance_report.report_type,
            ReportType::Body
        );
        assert_eq!(ai.base.my_reconnaissance_report.seek_position, position);
        assert_eq!(ai.base.seek_position, position);
        assert_eq!(ai.base.when_does_timer_ring, 160);
    }

    #[test]
    fn whistling_child_runs_to_source_then_waits_at_arrival() {
        let (mut engine, assets, owner, target) = fixture();
        let position = engine.live_ai_position(target);
        let ai = engine.reporting_civilian_mut(owner);
        ai.base.current_state = AiState::Wondering;
        ai.base.current_substate = Substate::WonderingWatchingWhistling;
        ai.base.seek_position = position;
        let sim = crate::sim_rng::test_context();
        assert_eq!(
            engine.execute_friendly_behavior(
                &sim,
                &assets,
                owner,
                &Stimulus::new(StimulusType::EventTimer)
            ),
            Some(false)
        );
        assert_eq!(
            engine.friendly_brain(owner).base.current_substate,
            Substate::WonderingChildApproachingWhistling
        );
        assert_eq!(
            engine.friendly_brain(owner).base.last_goto_destination,
            position
        );
        assert_eq!(
            engine.execute_friendly_behavior(
                &sim,
                &assets,
                owner,
                &Stimulus::new(StimulusType::EventReachPoint)
            ),
            Some(false)
        );
        assert_eq!(
            engine.friendly_brain(owner).base.current_substate,
            Substate::DefaultChildApproachedWhistling
        );
        assert_eq!(engine.friendly_brain(owner).base.when_does_timer_ring, 200);
    }

    #[test]
    fn irrelevant_object_sighting_keeps_civilian_state_and_effects() {
        let (mut engine, assets, owner, target) = fixture();
        let before = bitcode::encode(engine.friendly_brain(owner));
        let mut stimulus = Stimulus::new(StimulusType::EventSeesObject);
        stimulus.info = StimulusInfo::Object(AiEntityHandle::new(target.index()));
        assert_eq!(
            engine.execute_friendly_behavior(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                &stimulus
            ),
            Some(false)
        );
        assert_eq!(bitcode::encode(engine.friendly_brain(owner)), before);
    }

    #[test]
    fn live_apple_escape_uses_single_direction_draw_and_authorized_geometry() {
        let (mut engine, _, owner, target) = fixture();
        engine.place(
            target,
            crate::coordinates::WorldPoint3D::new(600.0, 500.0, 0.0),
        );
        engine.reporting_civilian_mut(owner).base.antagonist =
            Some(AiEntityHandle::new(target.index()));
        crate::sim_rng::with_seed(1, |sim| {
            let (destination, draws) =
                crate::sim_rng::with_draw_trace(|| engine.live_child_flee_destination(sim, owner));
            let destination = destination.expect("open sector permits escape");
            assert!(destination.x < engine.live_ai_position(owner).x);
            assert_eq!(draws, vec![crate::sim_rng::RngSite::CivilianPanicDirection]);
        });
    }

    #[test]
    #[should_panic(expected = "child chaser")]
    fn apple_chase_rejects_missing_chaser() {
        let (mut engine, assets, owner, _) = fixture();
        assert_eq!(
            engine.execute_friendly_behavior(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                &Stimulus::with_human(StimulusType::CallYouJustWait, u32::MAX),
            ),
            Some(false)
        );
    }

    #[test]
    fn civilian_panic_and_net_release_retain_threat_center_without_a_door() {
        for event in [StimulusType::EventPanic, StimulusType::EventNetAway] {
            let (mut engine, assets, owner, target) = fixture();
            let position = engine.live_ai_position(target);
            engine.reporting_civilian_mut(owner).base.seek_position = position;
            let stimulus = Stimulus::with_position(event, position);
            assert_eq!(
                engine.execute_friendly_behavior(
                    &crate::sim_rng::test_context(),
                    &assets,
                    owner,
                    &stimulus
                ),
                Some(false)
            );
            let ai = engine.friendly_brain(owner);
            assert!(!ai.base.directed_panic);
            assert_eq!(ai.base.panic_center_x, position.x);
            assert_eq!(ai.base.panic_center_y, position.y);
        }
    }

    #[test]
    fn fleeing_view_refreshes_panic_and_limits_repeated_sightings() {
        for (substate, count, next_count) in [
            (Substate::FleeingHiding, 7, 8),
            (Substate::FleeingRunToDoor, 0, 1),
            (Substate::FleeingRunToDoor, 7, 7),
        ] {
            let (mut engine, assets, owner, target) = fixture();
            let ai = engine.reporting_civilian_mut(owner);
            ai.base.current_state = AiState::Fleeing;
            ai.base.current_substate = substate;
            ai.fleeing_seen_enemy_counter = count;
            assert_eq!(
                engine.execute_friendly_behavior(
                    &crate::sim_rng::test_context(),
                    &assets,
                    owner,
                    &Stimulus::with_human(StimulusType::EventView, target.index())
                ),
                Some(false)
            );
            assert_eq!(
                engine.friendly_brain(owner).fleeing_seen_enemy_counter,
                next_count
            );
            if next_count != count {
                let position = engine.live_ai_position(target);
                let ai = engine.friendly_brain(owner);
                assert_eq!(ai.base.panic_center_x, position.x);
                assert_eq!(ai.base.panic_center_y, position.y);
                assert!(!ai.base.directed_panic);
            }
        }
    }

    #[test]
    fn hiding_completion_resets_sight_counter_before_a_new_live_threat() {
        let (mut engine, assets, owner, target) = fixture();
        let ai = engine.reporting_civilian_mut(owner);
        ai.base.current_state = AiState::Fleeing;
        ai.base.current_substate = Substate::FleeingHiding;
        ai.fleeing_seen_enemy_counter = 7;
        ai.base.launch_timer(0, 100);
        let sim = crate::sim_rng::test_context();
        engine.execute_ai_callback(
            &sim,
            &assets,
            owner,
            &Stimulus::new(StimulusType::EventTimer),
        );
        assert_eq!(engine.friendly_brain(owner).fleeing_seen_enemy_counter, 0);
        assert_eq!(
            engine.friendly_brain(owner).base.current_state,
            AiState::Default
        );
        let ai = engine.reporting_civilian_mut(owner);
        ai.base.current_state = AiState::Fleeing;
        ai.base.current_substate = Substate::FleeingRunToDoor;
        assert_eq!(
            engine.execute_friendly_behavior(
                &sim,
                &assets,
                owner,
                &Stimulus::with_human(StimulusType::EventView, target.index())
            ),
            Some(false)
        );
        assert_eq!(engine.friendly_brain(owner).fleeing_seen_enemy_counter, 1);
        let position = engine.live_ai_position(target);
        assert_eq!(engine.friendly_brain(owner).base.panic_center_x, position.x);
        assert_eq!(engine.friendly_brain(owner).base.panic_center_y, position.y);
        assert!(!engine.friendly_brain(owner).base.directed_panic);
    }
}
