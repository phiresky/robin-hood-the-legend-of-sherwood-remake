use super::*;
use crate::ai::{
    AiState, DutyFlags, EmoticonType, GotoFlags, SeekPoint, Stimulus, StimulusType, Substate,
};
use crate::ai_enemy::{SeekFlags, UNDEFINED_DIRECTION};
use crate::parameters_ai;
#[cfg(test)]
use crate::sim_rng::SimulationContext;

impl EngineInner {
    #[cfg(test)]
    pub(in crate::engine) fn execute_ai_seeking_event(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        AiOwnerCtx::new(self, sim, assets, owner).execute_ai_seeking_event(stimulus)
    }

    fn seek_event_timer(&mut self, owner: EntityId, duration: u32) {
        let frame = self.control.frame_counter;
        self.seek_enemy_mut(owner)
            .base
            .launch_timer(duration, frame);
    }

    fn current_live_seekpoint(&self, owner: EntityId) -> &SeekPoint {
        let ai = self.seek_enemy(owner);
        let id = ai
            .actual_seek_point
            .expect("seek operation requires actual point");
        match id {
            1111 => ai.personal_seek_point_1.as_ref(),
            2222 => ai.personal_seek_point_2.as_ref(),
            _ => self.ai.global.seek_points.get(usize::from(id)),
        }
        .expect("actual seek point must resolve")
    }
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_ai_seeking_event(
        &mut self,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        use StimulusType::*;
        use Substate::*;
        if !stimulus.stimulus_type.is_expected_class() {
            return Option::None;
        }
        let substate = self.engine.seek_enemy(self.owner).base.current_substate;
        if !matches!(
            substate,
            SeekingSeekpoint
                | SeekingSeekpointWatching
                | SeekingSeekpointWatchingSidewards
                | SeekingSeekpointPassedAmbushPointLeft
                | SeekingSeekpointPassedAmbushPointRight
                | SeekingSeekpointCheckingAmbushPoint
                | SeekingGotStopEvent
                | SeekingNet
                | DefaultPatrolEnroute
                | DefaultPatrolEnrouteRunning
                | DefaultPatrolEnrouteWaiting
                | DefaultGotoChief
                | DefaultPatrolChiefReturnToPatrol
        ) {
            return Option::None;
        }
        match (substate, stimulus.stimulus_type) {
            (SeekingSeekpoint, EventReachPoint) => self.execute_seekpoint_arrival(),
            (SeekingSeekpointWatching, EventTimer) => {
                self.duty_set_state(AiState::Seeking, SeekingSeekpointWatchingSidewards);
                let direction =
                    if crate::sim_rng::u32(self.sim, crate::sim_rng::RngSite::EnemySeekLook, 0..2)
                        != 0
                    {
                        crate::ai::LookDirection::LeftRight
                    } else {
                        crate::ai::LookDirection::RightLeft
                    };
                self.execute_ai_look_sidewards(direction);
            }
            (SeekingSeekpointWatchingSidewards, EventDone | EventTimer) => {
                if let Some(&direction) = self
                    .engine
                    .seek_enemy(self.owner)
                    .seek_point_view_directions
                    .first()
                {
                    self.engine
                        .seek_enemy_mut(self.owner)
                        .seek_point_view_directions
                        .remove(0);
                    self.duty_face_direction(direction);
                    self.engine.seek_enemy_mut(self.owner).base.number_of_looks = 0;
                    self.duty_set_state(AiState::Seeking, SeekingSeekpointWatching);
                    self.engine
                        .seek_event_timer(self.owner, parameters_ai::AI_SEEKPOINT_LOOK_TIME as u32);
                } else {
                    self.execute_ai_seek_next_point();
                }
            }
            (
                SeekingSeekpointPassedAmbushPointLeft | SeekingSeekpointPassedAmbushPointRight,
                EventReachPoint,
            ) => {
                self.duty_set_state(AiState::Seeking, SeekingSeekpoint);
                self.execute_ai_callback(&Stimulus::new(EventReachPoint));
            }
            (
                SeekingSeekpointPassedAmbushPointLeft | SeekingSeekpointPassedAmbushPointRight,
                EventTimer,
            ) => {
                self.stop_ai_owner();
                self.duty_set_state(AiState::Seeking, SeekingSeekpointCheckingAmbushPoint);
                self.execute_ai_look_sidewards(
                    if substate == SeekingSeekpointPassedAmbushPointLeft {
                        crate::ai::LookDirection::Left
                    } else {
                        crate::ai::LookDirection::Right
                    },
                );
            }
            (SeekingSeekpointCheckingAmbushPoint, EventDone) => {
                self.duty_set_state(AiState::Seeking, SeekingSeekpoint);
                let flags = if self
                    .engine
                    .seek_enemy(self.owner)
                    .seek_flags
                    .contains(SeekFlags::WALKING)
                {
                    GotoFlags::empty()
                } else {
                    GotoFlags::RUN
                };
                let position = self.engine.current_live_seekpoint(self.owner).position;
                self.duty_go_to(position, flags);
            }
            (SeekingGotStopEvent, EventTimer) => {
                let ai = self.engine.seek_enemy_mut(self.owner);
                if let Some(path) = ai.base.alert_path_id
                    && !ai.changed_to_alert_path
                {
                    ai.changed_to_alert_path = true;
                    ai.base.patrol_path =
                        crate::ai::PatrolPath::new(path, &self.assets.navigation.hiking_paths);
                    ai.base.has_patrol_path = true;
                }
                ai.base.set_emoticon(EmoticonType::QuestionMark);
                self.duty_set_state(AiState::Wondering, WonderingLooking1);
                self.engine.seek_event_timer(self.owner, 30);
            }
            (SeekingNet, EventTimer | EventReachPoint) => {
                self.execute_seeking_net(stimulus.stimulus_type)
            }
            (DefaultPatrolEnroute | DefaultPatrolEnrouteRunning, EventReachPoint) => {
                let direction = self.engine.seek_enemy(self.owner).base.patrol_direction;
                if direction
                    != self
                        .engine
                        .expect_entity(self.owner, "patrol arrival")
                        .element_data()
                        .direction() as u16
                {
                    self.duty_face_direction(direction);
                }
                self.duty_set_state(AiState::Default, DefaultPatrolEnrouteWaiting);
                self.engine.seek_event_timer(self.owner, 200);
            }
            (DefaultPatrolEnrouteWaiting, EventTimer) => {
                let chief = self
                    .engine
                    .seek_enemy(self.owner)
                    .base
                    .patrol_chief
                    .expect("waiting patrol follower requires chief");
                let state = self.engine.ai(chief, "patrol chief state").current_state;
                if matches!(state, AiState::Default | AiState::Wondering) {
                    self.engine.seek_event_timer(self.owner, 200);
                } else {
                    self.execute_ai_return_to_duty(DutyFlags::empty());
                }
            }
            (DefaultGotoChief, EventReachPoint) => {
                if let Some(chief) = self.engine.seek_enemy(self.owner).base.patrol_chief {
                    let position = self.engine.live_ai_position(chief);
                    let elevation = self
                        .engine
                        .expect_entity(chief, "patrol chief facing")
                        .position_iface()
                        .get_elevation() as i16;
                    let body = self
                        .engine
                        .expect_entity(self.owner, "patrol follower facing")
                        .element_data()
                        .position();
                    let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                        position.x - body.x,
                        (position.y - (body.y - body.z)) + (elevation as f32 - body.z),
                    );
                    self.duty_face_direction(direction as u16);
                    self.duty_set_state(AiState::Default, DefaultPatrolEnrouteWaiting);
                    self.engine.seek_event_timer(self.owner, 200);
                } else {
                    self.execute_ai_return_to_duty(DutyFlags::empty());
                }
            }
            (DefaultPatrolChiefReturnToPatrol, EventReachPoint) => {
                self.execute_ai_return_to_duty(DutyFlags::empty())
            }
            _ => {}
        }

        Some(false)
    }

    fn execute_seekpoint_arrival(&mut self) {
        if self
            .engine
            .seek_enemy(self.owner)
            .actual_seek_point
            .is_none()
        {
            return;
        }
        self.engine
            .seek_enemy_mut(self.owner)
            .seek_point_view_directions
            .clear();
        let count = self
            .engine
            .current_live_seekpoint(self.owner)
            .directions
            .len();
        for index in 0..count {
            let direction = self.engine.current_live_seekpoint(self.owner).directions[index];
            let facing = self
                .engine
                .expect_entity(self.owner, "seek arrival facing")
                .element_data()
                .direction();
            let relative = ((i32::from(direction) + 16 - i32::from(facing)) ^ 8) & 15;
            if matches!(relative, 15 | 0 | 1) {
                continue;
            }
            let insertion = crate::sim_rng::usize(
                self.sim,
                crate::sim_rng::RngSite::EnemySeekDirectionShuffle,
                0..=self
                    .engine
                    .seek_enemy(self.owner)
                    .seek_point_view_directions
                    .len(),
            );
            self.engine
                .seek_enemy_mut(self.owner)
                .seek_point_view_directions
                .insert(insertion, direction);
        }
        if let Some(&direction) = self
            .engine
            .seek_enemy(self.owner)
            .seek_point_view_directions
            .first()
        {
            self.engine
                .seek_enemy_mut(self.owner)
                .seek_point_view_directions
                .remove(0);
            self.duty_set_state(AiState::Seeking, Substate::SeekingSeekpointWatching);
            self.duty_face_direction(direction);
            self.engine
                .seek_event_timer(self.owner, parameters_ai::AI_SEEKPOINT_LOOK_TIME as u32);
        } else {
            self.execute_ai_seek_next_point();
        }
    }

    fn execute_seeking_net(&mut self, event: StimulusType) {
        let handle = self
            .engine
            .seek_enemy(self.owner)
            .base
            .detected_body
            .expect("net rescue requires body");
        let body = self
            .engine
            .expect_human_id_for_ai_handle(handle.get(), "net rescue body");
        let stuck = self
            .engine
            .expect_entity(body, "net rescue body")
            .human_data()
            .expect("net victim must be human")
            .stuck_under_nets_counter
            > 0;
        if event == StimulusType::EventTimer {
            if !stuck
                && self.engine.npc_is_detecting_human(
                    self.assets,
                    self.owner,
                    body,
                    self.engine.control.frame_counter,
                )
            {
                self.execute_ai_return_to_duty(DutyFlags::empty());
            } else {
                self.engine.seek_event_timer(self.owner, 10);
            }
        } else if !stuck {
            self.execute_ai_return_to_duty(DutyFlags::empty());
        } else if self
            .engine
            .expect_entity(self.owner, "net rescuer")
            .soldier_data()
            .is_some_and(|s| s.rider)
        {
            self.execute_ai_seek_area(
                self.engine.live_ai_position(self.owner),
                parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                SeekFlags::BODY_SEEK,
                UNDEFINED_DIRECTION,
            );
        } else {
            self.duty_set_state(AiState::Seeking, Substate::SeekingTakingNet);
            let net = self
                .engine
                .seek_enemy(self.owner)
                .base
                .interesting_object
                .map(|handle| EntityId::Net(crate::entity_id::NetId(handle.get())));
            if let Some(net) = net
                && self
                    .engine
                    .get_entity(net)
                    .is_some_and(|entity| entity.is_active())
            {
                self.stop_ai_owner();
                let mut sequence = crate::sequence::Sequence::new();
                for step in 1..=4 {
                    sequence.append_element(crate::sequence::SequenceElement::new_interaction(
                        step,
                        crate::element::Command::SearchCmd,
                        Some(self.owner),
                        Option::None,
                    ));
                }
                sequence.append_element(crate::sequence::SequenceElement::new_interaction(
                    5,
                    crate::element::Command::Take,
                    Some(self.owner),
                    Some(net),
                ));
                self.engine.launch_sequence(self.sim, self.assets, sequence);

                self.engine
                    .seek_enemy_mut(self.owner)
                    .base
                    .set_emoticon(EmoticonType::None);
            }
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::ai::{AiEntityHandle, AlertLevel, PathId, PatrolPath};
    use crate::coordinates::{MapPoint, WorldPoint3D};

    pub(in crate::engine::ai) fn fixture() -> (EngineInner, LevelAssets, EntityId, EntityId) {
        let mut engine = EngineInner::new();
        engine.control.frame_counter = 100;
        engine.world.fast_grid_mut().size_map(128, 128);
        engine.world.fast_grid_mut().allocate_layers(1);
        let index = engine.world.fast_grid_mut().add_sector(
            crate::engine::test_support::square_sector(
                1,
                0,
                MapPoint::new(0.0, 0.0),
                MapPoint::new(3000.0, 3000.0),
            ),
            0,
        );
        let sector = crate::ai::SectorHandle::new(1)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap());
        let owner =
            engine.add_test_entity(crate::engine::test_support::actors::make_test_ai_soldier(
                crate::element::Camp::Lacklandists,
            ));
        let target =
            engine.add_test_entity(crate::engine::test_support::actors::make_test_ai_soldier(
                crate::element::Camp::Lacklandists,
            ));
        for (id, x) in [(owner, 100.0), (target, 200.0)] {
            let entity = engine.get_entity_mut(id).unwrap();
            entity
                .element_data_mut()
                .set_position(WorldPoint3D::new(x, 100.0, 0.0));
            entity.element_data_mut().set_sector(Some(sector));
            entity.element_data_mut().active = true;
            entity.npc_data_mut().unwrap().life_points = 50;
            entity
                .position_iface_mut()
                .set_move_box(crate::coordinates::MoveBox::from_corners(
                    crate::coordinates::MapVec::new(-10.0, -5.0),
                    crate::coordinates::MapVec::new(10.0, 5.0),
                ));
        }
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.scripts.mission = Some(crate::engine::test_support::asm::empty_mission_script(
            "seeking_events.scs",
        ));
        let position = engine.live_ai_position(owner);
        let ai = engine.seek_enemy_mut(owner);
        ai.base.initial_position = position;
        (engine, assets, owner, target)
    }

    #[test]
    fn stopped_search_adopts_the_live_alert_path_and_preserves_yellow_alert() {
        for with_path in [false, true] {
            let (mut engine, mut assets, owner, _) = fixture();
            assets.navigation.hiking_paths = std::sync::Arc::new(vec![
                crate::level_data::RawHikingPath { waypoints: vec![] },
                crate::level_data::RawHikingPath { waypoints: vec![] },
            ]);
            let ai = engine.seek_enemy_mut(owner);
            ai.base.current_state = AiState::Seeking;
            ai.base.current_substate = Substate::SeekingGotStopEvent;
            ai.base.current_music_alert_status = AlertLevel::Yellow;
            ai.base.view_alert_status = AlertLevel::Yellow;
            if with_path {
                ai.base.alert_path_id = PathId::new(1);
                ai.base.patrol_path =
                    PatrolPath::new(PathId::new(0).unwrap(), &assets.navigation.hiking_paths);
                ai.base.has_patrol_path = true;
            }
            let sim = crate::sim_rng::test_context();
            assert_eq!(
                engine.execute_ai_seeking_event(
                    &sim,
                    &assets,
                    owner,
                    &Stimulus::new(StimulusType::EventTimer)
                ),
                Some(false)
            );
            let ai = engine.seek_enemy(owner);
            assert_eq!(ai.base.current_state, AiState::Wondering);
            assert_eq!(ai.base.current_substate, Substate::WonderingLooking1);
            assert_eq!(ai.base.current_music_alert_status, AlertLevel::Yellow);
            assert_eq!(ai.base.view_alert_status, AlertLevel::Yellow);
            assert_eq!(ai.base.current_emoticon_type, EmoticonType::QuestionMark);
            assert_eq!(ai.base.when_does_timer_ring, 130);
            assert_eq!(ai.changed_to_alert_path, with_path);
            if with_path {
                let path = ai.base.patrol_path.as_ref().unwrap();
                assert_eq!(path.hiking_path_index, PathId::new(1).unwrap());
                assert_eq!(path.current_waypoint_index, 0);
                assert!(path.forward && ai.base.has_patrol_path);
            } else {
                assert!(ai.base.patrol_path.is_none());
            }
        }
    }

    #[test]
    fn patrol_wait_reads_chief_state_at_each_event_without_a_primary_target() {
        let (mut engine, assets, owner, chief) = fixture();
        engine.seek_enemy_mut(owner).base.patrol_chief = Some(chief);
        engine.seek_enemy_mut(owner).base.primary_target = None;
        let sim = crate::sim_rng::test_context();
        for state in [AiState::Default, AiState::Wondering] {
            engine.seek_enemy_mut(chief).base.current_state = state;
            engine.seek_enemy_mut(owner).base.current_substate =
                Substate::DefaultPatrolEnrouteWaiting;
            assert_eq!(
                engine.execute_ai_seeking_event(
                    &sim,
                    &assets,
                    owner,
                    &Stimulus::new(StimulusType::EventTimer)
                ),
                Some(false)
            );
            assert_eq!(
                engine.seek_enemy(owner).base.current_substate,
                Substate::DefaultPatrolEnrouteWaiting
            );
            assert_eq!(engine.seek_enemy(owner).base.when_does_timer_ring, 300);
        }
    }

    #[test]
    fn chief_arrival_faces_the_live_elevation_before_waiting() {
        let (mut engine, assets, owner, chief) = fixture();
        engine
            .get_entity_mut(owner)
            .unwrap()
            .element_data_mut()
            .set_position(WorldPoint3D::new(1021.08, 2031.7904 + 27.71125, 27.71125));
        engine
            .get_entity_mut(owner)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(6);
        engine
            .get_entity_mut(chief)
            .unwrap()
            .element_data_mut()
            .set_position(WorldPoint3D::new(
                1033.5859,
                2036.767 + 25.100779,
                25.100779,
            ));
        let ai = engine.seek_enemy_mut(owner);
        ai.base.patrol_chief = Some(chief);
        ai.base.current_substate = Substate::DefaultGotoChief;
        let sim = crate::sim_rng::test_context();
        assert_eq!(
            engine.execute_ai_seeking_event(
                &sim,
                &assets,
                owner,
                &Stimulus::new(StimulusType::EventReachPoint)
            ),
            Some(false)
        );
        let turn = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .find(|element| {
                element.owner == Some(owner) && element.command == crate::element::Command::Turn
            })
            .expect("registered chief facing turn");
        assert!(matches!(
            turn.get_property(crate::sequence::Field::Direction),
            Some(crate::sequence::FieldValue::Integer(5))
        ));
        assert_eq!(
            engine.seek_enemy(owner).base.current_substate,
            Substate::DefaultPatrolEnrouteWaiting
        );
        assert_eq!(engine.seek_enemy(owner).base.when_does_timer_ring, 300);
    }

    #[test]
    fn net_arrival_enters_taking_state_even_when_the_net_object_has_disappeared() {
        let (mut engine, assets, owner, body) = fixture();
        engine
            .get_entity_mut(body)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .stuck_under_nets_counter = 1;
        let ai = engine.seek_enemy_mut(owner);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingNet;
        ai.base.detected_body = Some(AiEntityHandle::new(body.index()));
        ai.base.interesting_object = None;
        let sim = crate::sim_rng::test_context();
        assert_eq!(
            engine.execute_ai_seeking_event(
                &sim,
                &assets,
                owner,
                &Stimulus::new(StimulusType::EventTimer)
            ),
            Some(false)
        );
        assert_eq!(engine.seek_enemy(owner).base.when_does_timer_ring, 110);
        assert_eq!(
            engine.execute_ai_seeking_event(
                &sim,
                &assets,
                owner,
                &Stimulus::new(StimulusType::EventReachPoint)
            ),
            Some(false)
        );
        assert_eq!(
            engine.seek_enemy(owner).base.current_substate,
            Substate::SeekingTakingNet
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .current_order_for_actor(&engine.world.entities, owner)
                .is_none()
        );
    }

    #[test]
    fn seekpoint_arrival_filters_rear_directions_and_preserves_remaining_live_directions() {
        let (mut engine, assets, owner, _) = fixture();
        engine
            .get_entity_mut(owner)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(0);
        let position = engine.live_ai_position(owner);
        engine.ai.global.seek_points.push(SeekPoint {
            position,
            directions: vec![7, 8, 9, 2, 4],
            frame_when_full_interest: 0,
            last_calculated_interest: 100,
            locked: true,
            id: 0,
        });
        let ai = engine.seek_enemy_mut(owner);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingSeekpoint;
        ai.actual_seek_point = Some(0);
        crate::sim_rng::with_seed(441, |sim| {
            assert_eq!(
                engine.execute_ai_seeking_event(
                    sim,
                    &assets,
                    owner,
                    &Stimulus::new(StimulusType::EventReachPoint)
                ),
                Some(false)
            );
        });
        let ai = engine.seek_enemy(owner);
        assert_eq!(ai.base.current_substate, Substate::SeekingSeekpointWatching);
        assert_eq!(ai.seek_point_view_directions.len(), 1);
        let turn = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .find(|element| {
                element.owner == Some(owner) && element.command == crate::element::Command::Turn
            })
            .expect("registered first search facing turn");
        let Some(crate::sequence::FieldValue::Integer(direction)) =
            turn.get_property(crate::sequence::Field::Direction)
        else {
            panic!("search turn requires authored direction")
        };
        let mut directions = vec![*direction as u16, ai.seek_point_view_directions[0]];
        directions.sort_unstable();
        assert_eq!(directions, vec![2, 4]);
        assert_eq!(
            ai.base.when_does_timer_ring,
            100 + parameters_ai::AI_SEEKPOINT_LOOK_TIME as u32
        );
    }

    #[test]
    fn owned_search_states_leave_unexpected_events_to_the_shared_dispatcher() {
        let (mut engine, assets, owner, _) = fixture();
        engine.seek_enemy_mut(owner).base.current_substate = Substate::SeekingNet;
        let sim = crate::sim_rng::test_context();
        for event in [
            StimulusType::EventView,
            StimulusType::EventReturnToDuty,
            StimulusType::EventCouldntReachPoint,
        ] {
            assert_eq!(
                engine.execute_ai_seeking_event(&sim, &assets, owner, &Stimulus::new(event)),
                None
            );
        }
    }
}
