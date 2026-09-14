use super::*;
use crate::ai::{
    AiEntityHandle, AiState, DutyFlags, GotoFlags, Position, Remark, ReportType, SpeechFlags,
    Stimulus, StimulusInfo, StimulusType, Substate,
};
use crate::sim_rng::SimulationContext;

impl EngineInner {
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
            EventAfterScriptGoOn => {
                if !self
                    .friendly_brain(owner)
                    .base
                    .outbox
                    .reentrant
                    .engine_drains_after_script_go_on
                {
                    self.execute_civilian_after_script(sim, assets, owner);
                }
            }
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
                            self.reporting_civilian_mut(owner)
                                .base
                                .say_with_flags(Remark::CivPanic, SpeechFlags::HOUSE);
                            self.drain_direct_ai_owner_boundary(sim, owner, assets);
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
                self.reporting_civilian_mut(owner).panic_from_point_at(
                    position,
                    crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
                );
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
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
                self.reporting_civilian_mut(owner).panic_from_point_at(
                    position,
                    crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
                );
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
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

    pub(in crate::engine) fn execute_civilian_after_script(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let ai = self.friendly_brain(owner);
        if ai.base.current_state != AiState::Default {
            return;
        }
        if ai
            .base
            .patrol_path
            .as_ref()
            .and_then(|path| path.current_waypoint(&assets.navigation.hiking_paths))
            .is_none()
        {
            self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
            return;
        }
        self.reporting_civilian_mut(owner)
            .base
            .patrol_path
            .as_mut()
            .expect("civilian path")
            .advance();
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Default,
            Substate::DefaultEnroute,
        );
        let ai = self.friendly_brain(owner);
        let path = ai
            .base
            .patrol_path
            .as_ref()
            .expect("civilian route after state callback");
        let waypoint = path
            .current_waypoint(&assets.navigation.hiking_paths)
            .expect("civilian waypoint after state callback");
        let destination = Position {
            x: waypoint.x as f32,
            y: waypoint.y as f32,
            sector: assets.navigation.hiking_waypoint_sector(
                usize::from(path.hiking_path_index),
                usize::from(path.current_waypoint_index),
                waypoint.sector,
            ),
            level: waypoint.level,
        };
        let flags = ai.base.default_path_walking_flags;
        self.duty_go_to(sim, assets, owner, destination, flags);
    }

    fn civilian_timer(&mut self, owner: EntityId, duration: u32) {
        let frame = self.control.frame_counter;
        self.reporting_civilian_mut(owner)
            .base
            .launch_timer(duration, frame);
    }

    fn civilian_stop(&mut self, sim: &SimulationContext, assets: &LevelAssets, owner: EntityId) {
        self.reporting_civilian_mut(owner).base.stop_all();
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
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
        let body = self
            .expect_entity(owner, "civilian facing owner")
            .element_data()
            .position();
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
            position.x - body.x,
            (position.y - (body.y - body.z)) + (elevation as f32 - body.z),
        );
        self.duty_face_direction(sim, assets, owner, direction as u16);
    }

    fn civilian_panic_from_human(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: EntityId,
    ) {
        let position = self.live_ai_position(target);
        self.reporting_civilian_mut(owner)
            .panic_from_point_at(position, crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8);
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
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
                self.reporting_civilian_mut(owner)
                    .base
                    .say(Remark::CivAdmiresRobin);
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
            }
            self.civilian_stop(sim, assets, owner);
            self.civilian_face_human(sim, assets, owner, target);
            self.civilian_timer(owner, crate::parameters_ai::AI_FIRST_LOOK_TIME as u32);
        } else if self.entity_data_in_building_sector(
            self.expect_entity(owner, "civilian viewer").element_data(),
        ) {
            self.reporting_civilian_mut(owner)
                .base
                .say_with_flags(Remark::CivPanic, SpeechFlags::HOUSE);
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
            self.reporting_civilian_mut(owner)
                .panic_undirected(crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8);
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
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
        self.reporting_civilian_mut(owner)
            .base
            .say(Remark::CivSeesBody);
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
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
                    self.reporting_civilian_mut(owner)
                        .base
                        .say(Remark::CivWhistling);
                    self.drain_direct_ai_owner_boundary(sim, owner, assets);
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
                CallYourTalk1 => self
                    .reporting_civilian_mut(owner)
                    .base
                    .say(Remark::CivChildChasedBySoldier),
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
            .set_sector(engine.get_entity(soldier).unwrap().element_data().sector());
        civilian.npc_data_mut().unwrap().life_points = 50;
        civilian.npc_data_mut().unwrap().ai_brain =
            crate::element::AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(0)));
        let owner = engine.add_test_entity(civilian);
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        let position = engine.live_ai_position(owner);
        let ai = engine.reporting_civilian_mut(owner);
        ai.base.initial_position = position;
        ai.base.special_action = true;
        (engine, assets, owner, soldier)
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
        engine
            .get_entity_mut(target)
            .unwrap()
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(600.0, 500.0, 0.0));
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
    fn civilian_panic_and_net_release_keep_directed_centers() {
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
            assert!(ai.base.directed_panic);
            assert_eq!(ai.base.panic_center_x, position.x);
            assert_eq!(ai.base.panic_center_y, position.y);
        }
    }

    #[test]
    fn fleeing_view_refreshes_directed_panic_and_limits_repeated_sightings() {
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
                assert!(engine.friendly_brain(owner).base.directed_panic);
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
        assert!(engine.friendly_brain(owner).base.directed_panic);
    }
}
