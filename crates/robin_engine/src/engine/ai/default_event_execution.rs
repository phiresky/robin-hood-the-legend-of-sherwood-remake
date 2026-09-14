//! Default and sleeping decisions executed against live actor state.
use super::*;
use crate::ai::*;
use crate::sim_rng::SimulationContext;

impl EngineInner {
    fn is_default_expected_stimulus(kind: StimulusType) -> bool {
        use StimulusType::*;
        matches!(
            kind,
            EventReachPoint
                | EventDone
                | EventTimer
                | EventSyncCharly
                | CallCoordinate
                | CallInstruction
                | CallReport
                | EventGaloppLoopEnd
                | EventMyTalk0
                | EventMyTalk1
                | EventMyTalk2
                | EventMyTalk3
                | CallYourTalk0
                | CallYourTalk1
                | CallYourTalk2
                | CallYourTalk3
        )
    }

    fn default_ai(&self, owner: EntityId) -> &AiController {
        self.world
            .entities
            .expect_ai_controller(owner, format_args!("default event owner"))
    }

    fn default_ai_mut(&mut self, owner: EntityId) -> &mut AiController {
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("default event owner"))
    }

    fn default_timer(&mut self, owner: EntityId, frames: u32) {
        let frame = self.control.frame_counter;
        self.default_ai_mut(owner).launch_timer(frames, frame);
    }

    fn default_bored_live(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> bool {
        let entity = self.expect_entity(owner, "default boredom owner");
        let ai = entity.ai_controller().expect("boredom requires AI");
        let bored_animation = entity
            .actor_data()
            .expect("boredom requires actor")
            .installed_order
            .is_some_and(|order| {
                order.order_type == crate::order::OrderType::WaitingUprightBoredRandom
            });
        if entity.enemy_ai().is_none()
            || ai.current_substate != Substate::DefaultOnPost
            || bored_animation
            || ai.likes_to_sit_around
            || ai.special_action
        {
            return false;
        }
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Default,
            Substate::DefaultOnPostLookingSidewards,
        );
        self.stop_ai_owner(sim, assets, owner);
        let direction =
            match crate::sim_rng::u32(sim, crate::sim_rng::RngSite::DefaultPostLook, 0..4) {
                0 => LookDirection::Left,
                1 => LookDirection::Right,
                2 => LookDirection::LeftRight,
                _ => LookDirection::RightLeft,
            };
        self.execute_ai_look_sidewards(owner, direction);

        true
    }

    pub(in crate::engine) fn execute_ai_default_event(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        if !Self::is_default_expected_stimulus(stimulus.stimulus_type) {
            return None;
        }
        if let Some(result) = self.execute_ai_common_expected_event(sim, assets, owner, stimulus) {
            return Some(result);
        }
        let kind = stimulus.stimulus_type;
        match self.default_ai(owner).current_substate {
            Substate::SleepingAwakening => {
                if matches!(kind, StimulusType::EventDone | StimulusType::EventTimer) {
                    let enemy = self.seek_enemy_mut(owner);
                    if let Some(path) = enemy.base.alert_path_id
                        && !enemy.changed_to_alert_path
                    {
                        enemy.changed_to_alert_path = true;
                        enemy.base.patrol_path =
                            PatrolPath::new(path, &assets.navigation.hiking_paths);
                        enemy.base.has_patrol_path = enemy.base.patrol_path.is_some();
                    }
                    enemy.base.set_emoticon(EmoticonType::QuestionMark);
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Wondering,
                        Substate::WonderingLooking1,
                    );
                    self.default_timer(owner, 30);
                }
            }
            Substate::DefaultOnPostLookingSidewards => {
                if kind == StimulusType::EventDone {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Default,
                        Substate::DefaultOnPost,
                    );
                    let frames = self.ai_bored_time(sim, owner);
                    self.default_timer(owner, u32::from(frames));
                }
            }
            Substate::DefaultLookingOfficerForAdvice => {
                if kind == StimulusType::EventTimer {
                    self.execute_specialized_ai_duty(sim, assets, owner, DutyFlags::empty());
                }
            }
            Substate::DefaultLookingShadow => {
                if kind == StimulusType::EventTimer {
                    if self.default_ai(owner).max_visibility > 0 {
                        self.default_timer(owner, 10);
                    } else {
                        self.execute_specialized_ai_duty(sim, assets, owner, DutyFlags::empty());
                    }
                }
            }
            Substate::DefaultScriptDriven => {}
            _ => return None,
        }
        Some(false)
    }

    pub(in crate::engine) fn execute_ai_common_expected_event(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        if !Self::is_default_expected_stimulus(stimulus.stimulus_type) {
            return None;
        }
        if let Some(result) = self.execute_ai_common_fleeing_event(sim, assets, owner, stimulus) {
            return Some(result);
        }
        let kind = stimulus.stimulus_type;
        match self.default_ai(owner).current_substate {
            Substate::DefaultGotoPost => {
                if kind == StimulusType::EventReachPoint {
                    let direction = self.default_ai(owner).initial_view_direction;
                    self.duty_face_direction(sim, assets, owner, direction);
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Default,
                        Substate::DefaultGotoPostTurn,
                    );
                }
            }
            Substate::DefaultGotoPostTurn => {
                if kind == StimulusType::EventDone {
                    let ai = self.default_ai(owner);
                    let posture = if ai.likes_to_sit_around {
                        Some(crate::element::Posture::Sitting)
                    } else if ai.special_action {
                        Some(crate::element::Posture::Leisure)
                    } else {
                        None
                    };
                    if let Some(posture) = posture {
                        self.world
                            .entities
                            .expect_entity_mut(owner, format_args!("post arrival posture"))
                            .set_posture(posture);
                    }
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Default,
                        Substate::DefaultOnPost,
                    );
                    let frames = self.ai_bored_time(sim, owner);
                    self.default_timer(owner, u32::from(frames));
                }
            }
            Substate::DefaultOnPost => {
                if kind == StimulusType::EventTimer && !self.default_bored_live(sim, assets, owner)
                {
                    let frames = self.ai_bored_time(sim, owner);
                    self.default_timer(owner, u32::from(frames));
                }
            }
            Substate::DefaultGotoRoute => {
                assert_ne!(
                    kind,
                    StimulusType::EventReachPoint,
                    "route arrival must use its admitted Think boundary"
                );
            }
            Substate::DefaultGotoRouteTurn | Substate::DefaultEnroute => {
                let substate = self.default_ai(owner).current_substate;
                if (substate == Substate::DefaultGotoRouteTurn && kind == StimulusType::EventDone)
                    || (substate == Substate::DefaultEnroute
                        && kind == StimulusType::EventReachPoint)
                {
                    return Some(self.default_route_reached(sim, assets, owner));
                }
            }
            Substate::DefaultInMacro => {}
            Substate::DefaultInMacroWaitingForDone => {
                if kind == StimulusType::EventDone {
                    self.run_ai_macro(sim, assets, owner);
                }
            }
            _ => return None,
        }
        Some(true)
    }

    fn default_route_reached(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> bool {
        let Some(path) = self
            .default_ai(owner)
            .patrol_path
            .as_ref()
            .filter(|path| path.size != 0)
        else {
            self.execute_specialized_ai_duty(sim, assets, owner, DutyFlags::empty());
            return false;
        };
        // The selected waypoint remains the branch discriminator across partner callbacks.
        let script_waypoint = (path.hiking_path_index, path.current_waypoint_index);
        let command = &path
            .current_waypoint(&assets.navigation.hiking_paths)
            .expect("route waypoint")
            .command;
        let mut remaining = self.default_ai(owner).synchronizing_actors.len();
        let mut index = 0;
        while index < remaining {
            let partner = self.default_ai(owner).synchronizing_actors[index];
            let id = self.expect_entity_id_for_index(partner, "route synchronization partner");
            if self.default_ai(id).current_substate == Substate::DefaultSynchronizing {
                let waypoint = self
                    .default_ai(owner)
                    .patrol_path
                    .as_ref()
                    .expect("synchronization route")
                    .current_waypoint_index;
                let mut event = Stimulus::new(StimulusType::EventSyncCharly);
                event.info = StimulusInfo::Index(waypoint.into());
                self.dispatch_think_with_drain(sim, id, &event, None, assets);
            }
            if self.default_ai(id).current_substate != Substate::DefaultSynchronizing {
                self.default_ai_mut(owner)
                    .synchronizing_actors
                    .remove(index);
                remaining -= 1;
            } else {
                index += 1;
            }
        }
        match command {
            crate::level_data::WaypointCommand::None => {
                self.default_ai_mut(owner)
                    .patrol_path
                    .as_mut()
                    .expect("simple route")
                    .advance();
                if self.default_bored_live(sim, assets, owner) {
                    return true;
                }
                if self
                    .default_ai(owner)
                    .patrol_path
                    .as_ref()
                    .expect("simple route")
                    .size
                    == 1
                {
                    let position = self.live_ai_position(owner);
                    let direction = self
                        .expect_entity(owner, "post direction")
                        .element_data()
                        .direction();
                    let vector =
                        crate::shadow_polygon::sector_to_direction((direction & 15) as i16);
                    let ai = self.default_ai_mut(owner);
                    ai.has_patrol_path = false;
                    ai.initial_position = position;
                    ai.initial_view_direction = crate::position_interface::vector_to_sector_0_to_15(
                        vector[0] * crate::position_interface::ASPECT_RATIO,
                        vector[1],
                    ) as u16;
                    self.execute_specialized_ai_duty(sim, assets, owner, DutyFlags::empty());
                } else {
                    let straight =
                        self.default_ai(owner).current_substate == Substate::DefaultEnroute;
                    self.default_walk_current_waypoint(
                        sim,
                        assets,
                        owner,
                        straight,
                        WillStopCaller::SimpleWaypoint,
                        true,
                    );
                }
            }
            crate::level_data::WaypointCommand::Script(_) => {
                let (path, waypoint) = script_waypoint;
                self.execute_ai_waypoint_script(sim, owner, assets, path, waypoint);
            }
            crate::level_data::WaypointCommand::Macro(_) => {
                // Macro data is fetched again after synchronization, like the path pointer.
                let path = self
                    .default_ai(owner)
                    .patrol_path
                    .as_ref()
                    .expect("macro route");
                let crate::level_data::WaypointCommand::Macro(data) = &path
                    .current_waypoint(&assets.navigation.hiking_paths)
                    .expect("macro waypoint")
                    .command
                else {
                    panic!("macro waypoint changed command kind during synchronization");
                };
                if self.default_ai_mut(owner).prepare_waypoint_macro(sim, data) {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Default,
                        Substate::DefaultInMacro,
                    );
                    self.default_ai_mut(owner).macro_started_in_this_frame = true;
                    self.run_ai_macro(sim, assets, owner);
                } else {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Default,
                        Substate::DefaultEnroute,
                    );
                    let path = self
                        .default_ai_mut(owner)
                        .patrol_path
                        .as_mut()
                        .expect("macro route continuation");
                    if path.size <= 1 {
                        self.default_ai_mut(owner).already_on_point = true;
                    } else {
                        path.advance();
                        self.default_walk_current_waypoint(
                            sim,
                            assets,
                            owner,
                            true,
                            WillStopCaller::ProceedOnPath,
                            false,
                        );
                    }
                }
            }
        }
        true
    }

    fn default_walk_current_waypoint(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        straight: bool,
        caller: WillStopCaller,
        change_state: bool,
    ) {
        let frame = self.control.frame_counter;
        let creation = self.world.original_creation_order(owner);
        let ai = self.default_ai_mut(owner);
        let path = ai.patrol_path.as_ref().expect("route movement path");
        let waypoint = path
            .current_waypoint(&assets.navigation.hiking_paths)
            .expect("route movement waypoint");
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
        let mut flags = ai.default_path_walking_flags;
        if !ai.will_stop_at_next_waypoint_at(
            sim,
            &assets.navigation.hiking_paths,
            frame,
            Some(creation),
            caller,
        ) {
            flags |= GotoFlags::DONT_STOP;
        }
        if straight {
            flags |= GotoFlags::STRAIGHT;
        }
        if change_state {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Default,
                Substate::DefaultEnroute,
            );
        }
        self.duty_go_to(sim, assets, owner, destination, flags);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::test_support::{actors::make_test_ai_soldier, ensure_ordinary_sector};

    fn fixture() -> (EngineInner, LevelAssets, EntityId) {
        let mut engine = EngineInner::new();
        let sector = ensure_ordinary_sector(&mut engine, 1, 0);
        let mut entity = make_test_ai_soldier(crate::element::Camp::Lacklandists);
        entity.element_data_mut().set_sector(Some(sector));
        let owner = engine.add_test_entity(entity);
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        (engine, assets, owner)
    }

    #[test]
    fn common_default_returns_true_even_when_expected_event_needs_no_action() {
        let (mut engine, assets, owner) = fixture();
        let sim = crate::sim_rng::test_context();
        for substate in [
            Substate::DefaultGotoPost,
            Substate::DefaultGotoPostTurn,
            Substate::DefaultGotoRoute,
            Substate::DefaultGotoRouteTurn,
            Substate::DefaultOnPost,
            Substate::DefaultEnroute,
            Substate::DefaultInMacro,
            Substate::DefaultInMacroWaitingForDone,
        ] {
            let ai = engine.default_ai_mut(owner);
            ai.current_state = AiState::Default;
            ai.current_substate = substate;
            assert_eq!(
                engine.execute_ai_default_event(
                    &sim,
                    &assets,
                    owner,
                    &Stimulus::new(StimulusType::CallYourTalk0)
                ),
                Some(true),
                "{substate:?}"
            );
            assert_eq!(engine.default_ai(owner).current_substate, substate);
        }
    }

    #[test]
    fn default_and_sleeping_leave_perception_and_recovery_to_outer_dispatch() {
        let (mut engine, assets, owner) = fixture();
        let sim = crate::sim_rng::test_context();
        for substate in [
            Substate::DefaultOnPost,
            Substate::DefaultGotoRouteTurn,
            Substate::DefaultInMacroWaitingForDone,
            Substate::DefaultOnPostLookingSidewards,
            Substate::DefaultLookingOfficerForAdvice,
            Substate::DefaultLookingShadow,
            Substate::SleepingAwakening,
        ] {
            let ai = engine.default_ai_mut(owner);
            ai.current_state = substate.ai_state_family().unwrap();
            ai.current_substate = substate;
            ai.max_visibility = 1;
            ai.launch_timer(123, 0);
            for event in [
                StimulusType::EventView,
                StimulusType::EventHear,
                StimulusType::EventSeesObject,
                StimulusType::EventSeesBody,
                StimulusType::EventCouldntReachPoint,
                StimulusType::EventLoseConsciousness,
                StimulusType::EventFitAgain,
                StimulusType::EventWaspAway,
                StimulusType::EventNetAway,
                StimulusType::CallLookThere,
            ] {
                assert_eq!(
                    engine.execute_ai_default_event(&sim, &assets, owner, &Stimulus::new(event)),
                    None,
                    "{substate:?} / {event:?}"
                );
                let ai = engine.default_ai(owner);
                assert_eq!(ai.current_substate, substate);
                assert!(ai.timer_is_running);
                assert_eq!(ai.when_does_timer_ring, 123);
            }
        }
    }

    #[test]
    fn shadow_timer_stays_specialized_and_returns_false() {
        let (mut engine, assets, owner) = fixture();
        engine.control.frame_counter = 71;
        let ai = engine.default_ai_mut(owner);
        ai.current_state = AiState::Default;
        ai.current_substate = Substate::DefaultLookingShadow;
        ai.max_visibility = 1;
        assert_eq!(
            engine.execute_ai_default_event(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                &Stimulus::new(StimulusType::EventTimer)
            ),
            Some(false)
        );
        assert_eq!(engine.default_ai(owner).when_does_timer_ring, 81);
    }

    #[test]
    fn entering_fleeing_hiding_blinks_visible_enemies_for_redetection() {
        for substate in [Substate::FleeingRunToHide, Substate::FleeingRunToDoor] {
            let (mut engine, assets, owner) = fixture();
            let ai = engine.default_ai_mut(owner);
            ai.current_state = AiState::Fleeing;
            ai.current_substate = substate;
            ai.directed_panic = true;
            ai.lasting_panic_runs = 0;
            engine
                .world
                .entities
                .expect_ai_actor_data_mut(owner, format_args!("visible enemy fixture"))
                .detectable_lists[crate::element::DetectableType::Enemy as usize]
                .push(crate::element::Detectable {
                    element: Some(owner),
                    detectable_type: crate::element::DetectableType::Enemy,
                    seen_now: true,
                    seen_last_frame: true,
                    ..Default::default()
                });
            engine.execute_ai_common_expected_event(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                &Stimulus::new(StimulusType::EventReachPoint),
            );
            let ai = engine.default_ai(owner);
            assert_eq!(ai.current_substate, Substate::FleeingHiding);
            assert!(ai.timer_is_running);
            let minimum = crate::parameters_ai::AI_MIN_PANIC_HIDING_TIME as u32;
            let limit = minimum + crate::parameters_ai::AI_DELTA_PANIC_HIDING_TIME as u32;
            assert!((minimum..limit).contains(&ai.when_does_timer_ring));
            let enemy = &engine
                .world
                .entities
                .expect_entity(owner, format_args!("hidden owner"))
                .ai_actor_data()
                .unwrap()
                .detectable_lists[crate::element::DetectableType::Enemy as usize][0];
            assert!(!enemy.seen_now);
            assert!(!enemy.seen_last_frame);
        }
    }
}

#[cfg(test)]
mod movement_tests {
    use super::*;
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::element::{ActionState, Command, InstalledActorOrder};
    use crate::engine::test_support::{actors::make_test_ai_soldier, square_sector};
    use crate::order::OrderType;
    use crate::sequence::{Field, FieldValue, SequenceElementData};

    fn fixture(animation: OrderType) -> (EngineInner, LevelAssets, EntityId) {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(256, 256);
        engine.world.fast_grid_mut().allocate_layers(1);
        let index = engine.world.fast_grid_mut().add_sector(
            square_sector(1, 0, MapPoint::new(0.0, 0.0), MapPoint::new(4000.0, 4000.0)),
            0,
        );
        let sector = crate::position_interface::SectorHandle::new(1)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap());
        let mut entity = make_test_ai_soldier(crate::element::Camp::Lacklandists);
        entity.element_data_mut().active = true;
        entity
            .element_data_mut()
            .set_sector_topology(Some(sector), sector.arena_index());
        entity
            .element_data_mut()
            .set_position(WorldPoint3D::new(100.0, 200.0, 0.0));
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(100.0, 200.0));
        entity.actor_data_mut().unwrap().installed_order = Some(InstalledActorOrder {
            order_id: std::num::NonZeroU32::new(1).unwrap(),
            order_type: animation,
        });
        let owner = engine.add_test_entity(entity);
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.enter_ai_think_frame(owner);
        (engine, assets, owner)
    }

    fn registered_turn(engine: &EngineInner, owner: EntityId, command: Command) -> u32 {
        let turn = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .find(|element| element.owner == Some(owner) && element.command == command)
            .expect("facing registers its turn");
        match turn.get_property(Field::Direction) {
            Some(FieldValue::Integer(direction)) => *direction,
            other => panic!("turn direction missing: {other:?}"),
        }
    }

    fn has_move(engine: &EngineInner, owner: EntityId) -> bool {
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.owner == Some(owner)
                    && matches!(element.data, SequenceElementData::Movement { .. })
            })
    }

    #[test]
    fn live_reaction_uses_hard_difficulty_modifier_without_drawing_randomness() {
        for (fixed, deadline) in [(false, 111), (true, 36)] {
            let (mut engine, _, owner) = fixture(OrderType::WaitingUpright);
            engine.control.frame_counter = 10;
            engine.control.sim_config.difficulty = crate::player_profile::DifficultyLevel::Hard;
            engine.seek_enemy_mut(owner).soldier_profile_iq = 50;
            let mut config = engine.control.sim_config;
            config.fix_hard_reaction_times = fixed;
            let sim = SimulationContext::with_seed_and_config(123, config);
            let before = sim.seed();
            engine.execute_ai_react_live(&sim, owner, 100);
            assert_eq!(engine.default_ai(owner).when_does_timer_ring, deadline);
            assert_eq!(
                sim.seed(),
                before,
                "reaction time is an intelligence/difficulty calculation"
            );
        }
    }

    #[test]
    fn live_body_delegation_uses_current_patrol_not_theoretical_membership() {
        for live_member in [false, true] {
            let (mut engine, mut assets, owner) = fixture(OrderType::WaitingUpright);
            let sector = engine
                .expect_entity(owner, "delegation sector")
                .element_data()
                .sector();
            let mut body = make_test_ai_soldier(crate::element::Camp::Lacklandists);
            body.element_data_mut()
                .set_position(WorldPoint3D::new(500.0, 200.0, 0.0));
            body.element_data_mut().set_sector(sector);
            body.human_data_mut().unwrap().unconscious = true;
            let body = engine.add_test_entity(body);
            let mut member = make_test_ai_soldier(crate::element::Camp::Lacklandists);
            member
                .element_data_mut()
                .set_position(WorldPoint3D::new(100.0, 250.0, 0.0));
            member.element_data_mut().set_sector(sector);
            let member = engine.add_test_entity(member);
            crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
            let body_position = engine.live_ai_position(body);
            let ai = engine.seek_enemy_mut(owner);
            ai.base.current_state = AiState::Seeking;
            ai.base.current_substate = Substate::SeekingBodyReactiontime;
            ai.soldier_profile_rank = crate::profiles::ProfileRank::Officer;
            ai.soldier_profile_initiative = 60;
            ai.base.detected_body = Some(AiEntityHandle::new(body.index()));
            ai.base
                .my_reconnaissance_report
                .update(ReportType::Body, body_position);
            ai.base.theoretical_patrol.push(member);
            if live_member {
                ai.base.patrol.push(member);
            }
            engine.execute_ai_body_reaction_timer(&crate::sim_rng::test_context(), &assets, owner);
            assert_eq!(
                engine.default_ai(owner).current_substate,
                if live_member {
                    Substate::SeekingOfficerLookingForSoldiers1
                } else {
                    Substate::SeekingBody
                }
            );
        }
    }

    #[test]
    fn live_goto_same_point_checks_installed_animation_for_both_speed_overloads() {
        for animation in [
            OrderType::WaitingUpright,
            OrderType::WaitingAlerted,
            OrderType::NonanimationEnd,
            OrderType::WaitingUprightBored,
            OrderType::RunningUpright,
            OrderType::AimingWithBow,
            OrderType::ExtractingArrowBow,
            OrderType::TransitionRunningUprightWaitingUpright,
        ] {
            for speed in [1.0, 1.5] {
                let (mut engine, assets, owner) = fixture(animation);
                let point = engine.live_ai_position(owner);
                engine.duty_go_to_speed(
                    &crate::sim_rng::test_context(),
                    &assets,
                    owner,
                    point,
                    GotoFlags::RUN,
                    speed,
                );
                let idle = matches!(
                    animation,
                    OrderType::WaitingUpright
                        | OrderType::WaitingAlerted
                        | OrderType::NonanimationEnd
                );
                assert_eq!(
                    engine.default_ai(owner).already_on_point,
                    idle,
                    "{animation:?}/{speed}"
                );
                assert_eq!(has_move(&engine, owner), !idle, "{animation:?}/{speed}");
            }
        }
    }

    #[test]
    fn live_go_near_checks_idle_shortcut_before_zero_tolerance() {
        for animation in [OrderType::NonanimationEnd, OrderType::RunningUpright] {
            let (mut engine, assets, owner) = fixture(animation);
            let mut point = engine.live_ai_position(owner);
            point.x += 1.0;
            point.y += 3.0;
            engine.duty_go_near(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                point,
                0,
                GotoFlags::RUN,
            );
            let idle = animation == OrderType::NonanimationEnd;
            assert_eq!(engine.default_ai(owner).already_on_point, idle);
            assert_eq!(has_move(&engine, owner), !idle);
        }
    }

    #[test]
    fn live_replayed_near_flags_keep_tolerance_and_destination() {
        for (dx, dy, arrived) in [(-5.8875, -19.3909, true), (100.0, 100.0, false)] {
            let (mut engine, assets, owner) = fixture(OrderType::StrikingRightSmalltalk);
            let mut point = engine.live_ai_position(owner);
            point.x += dx;
            point.y += dy;
            engine
                .default_ai_mut(owner)
                .stop_before_end_of_path_distance = 65;
            let flags = GotoFlags::NEAR | GotoFlags::SWORD;
            engine.duty_go_to(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                point,
                flags,
            );
            let ai = engine.default_ai(owner);
            assert_eq!(ai.already_on_point, arrived);
            assert_eq!(ai.last_goto_destination, point);
            assert_eq!(ai.last_goto_flags, flags);
            assert_eq!(has_move(&engine, owner), !arrived);
            if !arrived {
                assert!(
                    engine
                        .orders
                        .sequence_manager
                        .sequences_iter()
                        .flat_map(|sequence| sequence.elements.iter())
                        .any(|element| element.owner == Some(owner)
                            && matches!(
                                element.data,
                                SequenceElementData::Movement {
                                    tolerance: 65.0,
                                    ..
                                }
                            ))
                );
            }
        }
    }

    #[test]
    fn live_movement_puts_action_teardown_before_move_in_same_sequence() {
        for (state, prefix) in [
            (ActionState::MovingSword, Command::QuitSwordfight),
            (ActionState::Menacing, Command::StopMenace),
        ] {
            let (mut engine, assets, owner) = fixture(OrderType::RunningUpright);
            engine
                .world
                .entities
                .expect_entity_mut(owner, format_args!("movement action fixture"))
                .actor_data_mut()
                .unwrap()
                .action_state = state;
            let mut point = engine.live_ai_position(owner);
            point.x += 100.0;
            engine.duty_go_to(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                point,
                GotoFlags::RUN,
            );
            assert!(
                engine
                    .orders
                    .sequence_manager
                    .sequences_iter()
                    .any(|sequence| {
                        let commands: Vec<_> = sequence
                            .elements
                            .iter()
                            .filter(|element| element.owner == Some(owner))
                            .map(|element| element.command)
                            .collect();
                        commands
                            .windows(2)
                            .any(|pair| pair == [prefix, Command::Move])
                    }),
                "{state:?} must exit before its movement"
            );
        }
    }

    #[test]
    fn live_same_direction_facing_only_shortcuts_waiting_and_bored() {
        for state in [
            ActionState::Waiting,
            ActionState::Bored,
            ActionState::Moving,
            ActionState::MovingFast,
            ActionState::AimingWithBow,
            ActionState::HoldingShield,
            ActionState::Menacing,
        ] {
            let (mut engine, assets, owner) = fixture(OrderType::WaitingUpright);
            let entity = engine
                .world
                .entities
                .expect_entity_mut(owner, format_args!("facing action fixture"));
            entity.actor_data_mut().unwrap().action_state = state;
            let direction = entity.element_data().direction() as u16;
            engine.duty_face_direction(&crate::sim_rng::test_context(), &assets, owner, direction);
            let idle = matches!(state, ActionState::Waiting | ActionState::Bored);
            assert_eq!(engine.default_ai(owner).already_turned, idle, "{state:?}");
            if !idle {
                assert_eq!(
                    registered_turn(&engine, owner, Command::Turn),
                    u32::from(direction)
                );
            }
        }
    }

    #[test]
    fn live_direction_facing_preserves_all_authored_sectors() {
        for direction in 0..16 {
            let (mut engine, assets, owner) = fixture(OrderType::RunningUpright);
            engine
                .world
                .entities
                .expect_entity_mut(owner, format_args!("facing sector fixture"))
                .actor_data_mut()
                .unwrap()
                .action_state = ActionState::Moving;
            engine.duty_face_direction(&crate::sim_rng::test_context(), &assets, owner, direction);
            assert_eq!(
                registered_turn(&engine, owner, Command::Turn),
                u32::from(direction)
            );
        }
    }

    #[test]
    fn live_fast_position_facing_registers_fast_turn_with_resolved_direction() {
        let (mut engine, assets, owner) = fixture(OrderType::RunningUpright);
        engine
            .world
            .entities
            .expect_entity_mut(owner, format_args!("fast facing fixture"))
            .actor_data_mut()
            .unwrap()
            .action_state = ActionState::Moving;
        let mut point = engine.live_ai_position(owner);
        point.x += 100.0;
        engine.duty_face_position_signed_elevation(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            point,
            0,
            true,
        );
        assert_eq!(
            registered_turn(&engine, owner, Command::TurnFast),
            crate::position_interface::vector_to_sector_0_to_15_iso(100.0, 0.0) as u32
        );
    }

    #[test]
    fn live_ground_facing_measures_from_body_while_door_slot_overrides_ai_position() {
        let (mut engine, assets, owner) = fixture(OrderType::RunningUpright);
        engine.scripts.mission = Some(crate::engine::test_support::asm::empty_mission_script(
            "door_facing_test.scs",
        ));
        let entity = engine
            .world
            .entities
            .expect_entity_mut(owner, format_args!("door facing fixture"));
        entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
        let sector = entity.element_data().sector().unwrap();
        let gate =
            crate::gate::DoorIndex::new(engine.script_domains.interactables.doors.len() as u32)
                .unwrap();
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                point_in: MapPoint::new(500.0, 500.0),
                point_out: MapPoint::new(500.0, 500.0),
                sector_in: crate::sector::SectorNumber::new(1),
                sector_out: crate::sector::SectorNumber::new(1),
                sector_in_index: sector.arena_index(),
                sector_out_index: sector.arena_index(),
                ..Default::default()
            });
        let mut pass = crate::sequence::SequenceElement::new_movement(
            1,
            Command::PassDoor,
            Some(owner),
            OrderType::WalkingUpright,
        );
        let SequenceElementData::Movement {
            gate_id, direction, ..
        } = &mut pass.data
        else {
            unreachable!()
        };
        *gate_id = Some(gate);
        *direction = 1;
        let sequence = engine.orders.sequence_manager.launch_element(pass);
        engine.element_in_progress(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            &mut Vec::new(),
            sequence,
            0,
        );
        assert_eq!(
            engine.live_ai_position(owner).map_point(),
            MapPoint::new(500.0, 500.0)
        );
        engine.duty_face_position_ground(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            Position {
                x: 100.0,
                y: 100.0,
                sector: None,
                level: 0,
            },
        );
        assert_eq!(
            registered_turn(&engine, owner, Command::Turn),
            crate::position_interface::vector_to_sector_0_to_15_iso(0.0, -100.0) as u32
        );
    }

    #[test]
    fn live_noise_facing_preserves_recorded_elevation_and_null_origin_projection() {
        for (body, origin, layer, sector, elevation, expected) in [
            (
                (1023.0087, 2018.9961, 400.001),
                (1135.0, 1843.0),
                0,
                None,
                220,
                6,
            ),
            (
                (1023.0087, 2018.9961, 400.001),
                (1135.0, 1843.0),
                0,
                crate::position_interface::SectorHandle::new(7),
                220,
                6,
            ),
            (
                (317.8, 1196.001, 480.001_04),
                (341.819_34, 716.628_85),
                u16::MAX,
                None,
                480,
                0,
            ),
            (
                (1079.0, 2300.001, 150.001),
                (1092.9459, 2107.2961),
                u16::MAX,
                None,
                175,
                0,
            ),
        ] {
            let (mut engine, assets, owner) = fixture(OrderType::RunningUpright);
            let entity = engine
                .world
                .entities
                .expect_entity_mut(owner, format_args!("noise facing fixture"));
            entity
                .element_data_mut()
                .set_position(WorldPoint3D::new(body.0, body.1, body.2));
            entity
                .element_data_mut()
                .set_position_map_preserving_3d(MapPoint::new(body.0, body.1 - body.2));
            entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
            let point = Position {
                x: origin.0,
                y: origin.1,
                sector,
                level: layer,
            };
            let noise = Noise {
                origin: NoiseOrigin::from_position(point),
                noise_type: NoiseType::Zonk,
                volume: 1,
                elevation,
                element_id: 0,
            };
            engine.default_ai_mut(owner).seek_position = noise.origin.legacy_position();
            engine.alert_face_noise_position(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                &noise,
            );
            assert_eq!(registered_turn(&engine, owner, Command::Turn), expected);
        }
    }
}
