use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{AiState, AlertLevel, Position, StimulusInfo, Substate};
    use crate::engine::test_support::actors::make_test_ai_soldier;

    fn assembly_fixture(points: &[(f32, f32, f32)]) -> (EngineInner, LevelAssets, Vec<EntityId>) {
        let mut engine = EngineInner::new();
        let sector = crate::engine::test_support::ensure_ordinary_sector(&mut engine, 1, 0);
        let ids: Vec<_> = points
            .iter()
            .map(|&(x, y, z)| {
                let mut entity = make_test_ai_soldier(crate::element::Camp::Lacklandists);
                entity
                    .element_data_mut()
                    .set_position(crate::coordinates::WorldPoint3D::new(x, y, z));
                entity.element_data_mut().set_sector(Some(sector));
                entity.ai_actor_data_mut().unwrap().view_radius = 1000;
                entity.enemy_ai_mut().unwrap().base.current_state = AiState::Default;
                engine.add_test_entity(entity)
            })
            .collect();
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine
            .world
            .entities
            .expect_ai_controller_mut(ids[0], format_args!("patrol chief"))
            .theoretical_patrol = ids[1..].to_vec();
        (engine, assets, ids)
    }

    #[test]
    fn patrol_initialization_inserts_ties_first_and_leaves_odd_tail_unpaired() {
        let (mut engine, assets, ids) = assembly_fixture(&[
            (100.0, 100.0, 0.0),
            (101.0, 100.0, 0.0),
            (101.0, 100.0, 0.0),
            (105.0, 100.0, 0.0),
        ]);
        engine.initialize_patrol_for_npc(&assets, ids[0]);
        let chief = engine
            .expect_entity(ids[0], "chief")
            .ai_controller()
            .unwrap();
        assert_eq!(chief.patrol, [ids[2], ids[1], ids[3]]);
        assert!(chief.missed_patrol_members.is_empty());
        for &member in &ids[1..] {
            assert_eq!(
                engine
                    .expect_entity(member, "member")
                    .ai_controller()
                    .unwrap()
                    .patrol_chief,
                Some(ids[0])
            );
        }
    }

    #[test]
    fn patrol_initialization_keeps_unordered_distance_at_insertion_front() {
        let (mut engine, assets, ids) = assembly_fixture(&[
            (100.0, 100.0, 0.0),
            (101.0, 100.0, 0.0),
            (102.0, 100.0, 0.0),
            (103.0, 100.0, 0.0),
        ]);
        engine.place(
            ids[3],
            crate::coordinates::WorldPoint3D::new(f32::NAN, 100.0, 0.0),
        );
        engine.initialize_patrol_for_npc(&assets, ids[0]);
        assert_eq!(
            engine
                .expect_entity(ids[0], "chief")
                .ai_controller()
                .unwrap()
                .patrol,
            [ids[3], ids[1], ids[2]]
        );
        assert!(
            engine
                .expect_entity(ids[3], "unordered member")
                .human_data()
                .unwrap()
                .sorting_distance
                .is_nan()
        );
    }

    #[test]
    fn patrol_initialization_uses_raw_world_distance_and_live_door_position_for_pairs() {
        let (mut engine, assets, ids) = assembly_fixture(&[
            (100.0, 100.0, 0.0),
            (101.0, 100.0, 0.0),
            (102.0, 100.0, 0.0),
            (100.0, 100.0, 10.0),
        ]);
        engine.scripts.mission = Some(crate::engine::test_support::asm::empty_mission_script(
            "patrol.scs",
        ));
        let sector = engine.live_ai_position(ids[0]).sector.unwrap();
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                point_in: MapPoint::new(100.0, 99.0),
                point_out: MapPoint::new(100.0, 99.0),
                sector_in: crate::sector::SectorNumber::new(1),
                sector_out: crate::sector::SectorNumber::new(1),
                sector_in_index: sector.arena_index(),
                sector_out_index: sector.arena_index(),
                ..Default::default()
            });
        let mut pass = crate::sequence::SequenceElement::new_movement(
            1,
            crate::element::Command::PassDoor,
            Some(ids[2]),
            crate::order::OrderType::WalkingUpright,
        );
        let crate::sequence::SequenceElementData::Movement {
            gate_id, direction, ..
        } = &mut pass.data
        else {
            unreachable!()
        };
        *gate_id = Some(crate::gate::DoorIndex::new(0).unwrap());
        *direction = 1;
        let sequence = engine.orders.sequence_manager.insert_element(pass);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence);
        engine.select_sequence_element(ids[2], Some((sequence, 0)));
        engine.t_element_in_progress(&assets, sequence, 0);
        assert_eq!(
            engine.live_ai_position(ids[2]).map_point(),
            MapPoint::new(100.0, 99.0)
        );
        engine.initialize_patrol_for_npc(&assets, ids[0]);
        assert_eq!(
            engine
                .expect_entity(ids[0], "chief")
                .ai_controller()
                .unwrap()
                .patrol,
            [ids[2], ids[1], ids[3]]
        );
        let distances: Vec<_> = ids[1..]
            .iter()
            .map(|&id| {
                engine
                    .expect_entity(id, "member")
                    .human_data()
                    .unwrap()
                    .sorting_distance
            })
            .collect();
        assert_eq!(distances, [1.0, 4.0, 100.0]);
    }

    #[test]
    fn patrol_initialization_preserves_authored_los_and_missed_member_order() {
        let (mut engine, assets, ids) = assembly_fixture(&[
            (100.0, 100.0, 0.0),
            (110.0, 100.0, 0.0),
            (120.0, 100.0, 0.0),
            (130.0, 100.0, 0.0),
            (140.0, 100.0, 0.0),
        ]);
        engine.enemy_mut(ids[1]).base.current_state = AiState::Attacking;
        engine.set_active(ids[2], false);
        let dead = engine.ent_mut(ids[4]);
        dead.element_data_mut().active = false;
        dead.npc_data_mut().unwrap().life_points = 0;
        crate::sight_obstacle::begin_parity_visibility_capture();
        engine.initialize_patrol_for_npc(&assets, ids[0]);
        let queries = crate::sight_obstacle::take_parity_visibility_capture();
        assert_eq!(
            queries
                .iter()
                .map(|query| query.destination[0])
                .collect::<Vec<_>>(),
            [110.0, 130.0]
        );
        let chief = engine
            .expect_entity(ids[0], "chief")
            .ai_controller()
            .unwrap();
        assert_eq!(chief.patrol, [ids[3]]);
        assert_eq!(chief.missed_patrol_members, [ids[1], ids[2]]);
        assert_eq!(
            engine
                .expect_entity(ids[4], "dead member")
                .human_data()
                .unwrap()
                .sorting_distance,
            1600.0
        );
    }

    #[test]
    fn patrol_coordinate_commits_role_state_before_walk_and_run() {
        for (distance, attentive, expected_substate, expected_action) in [
            (
                45.0,
                true,
                Substate::DefaultPatrolEnroute,
                crate::order::OrderType::WalkingUpright,
            ),
            (
                60.0,
                true,
                Substate::DefaultPatrolEnrouteRunning,
                crate::order::OrderType::RunningUpright,
            ),
            (
                45.0,
                false,
                Substate::DefaultPatrolEnroute,
                crate::order::OrderType::WalkingUpright,
            ),
        ] {
            let mut engine = EngineInner::new();
            let (sector, _) = crate::engine::test_support::extra_engine_combat::square_sector_map(
                &mut engine,
                (128, 128),
                (2000.0, 2000.0),
            );
            let mut ids = Vec::new();
            for x in [100.0, 200.0] {
                let mut entity = make_test_ai_soldier(crate::element::Camp::Lacklandists);
                entity
                    .element_data_mut()
                    .set_position_map(MapPoint::new(x, 100.0));
                entity
                    .element_data_mut()
                    .set_position(crate::coordinates::WorldPoint3D::new(x, 100.0, 0.0));
                entity.element_data_mut().set_sector(Some(sector));
                entity.actor_data_mut().unwrap().action_state =
                    crate::element::ActionState::Waiting;
                ids.push(engine.add_test_entity(entity));
            }
            let [owner, chief] = ids.as_slice() else {
                unreachable!()
            };
            let (owner, chief) = (*owner, *chief);
            let mut assets = LevelAssets::new();
            crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
            let ai = engine
                .world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("coordinate test"));
            ai.base.patrol_chief = Some(chief);
            ai.base.current_state = AiState::Default;
            ai.base.current_substate = if attentive {
                Substate::DefaultOnPost
            } else {
                Substate::DefaultPatrolEnroute
            };
            ai.attentive = attentive;
            ai.will_be_attentive = attentive;
            engine.execute_ai_set_alert_status(
                &assets,
                owner,
                AlertLevel::Yellow,
                crate::ai::AlertFlags::empty(),
            );

            engine.execute_ai_coordinate_patrol(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                &StimulusInfo::Position(Position {
                    x: 100.0 + distance,
                    y: 100.0,
                    sector: Some(sector),
                    level: 0,
                }),
            );

            let ai = engine
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("coordinate test"));
            assert_eq!(ai.base.current_state, AiState::Default);
            assert_eq!(ai.base.current_substate, expected_substate);
            assert_eq!(ai.base.current_music_alert_status, AlertLevel::Green);
            assert_eq!(ai.base.view_alert_status, AlertLevel::Green);
            assert!(!ai.will_be_attentive);
            assert!(engine.orders.sequence_manager.sequences_iter().any(|sequence| {
                sequence.elements.iter().any(|element| element.owner == Some(owner)
                    && matches!(element.data, crate::sequence::SequenceElementData::Movement { action, .. } if action == expected_action))
            }), "patrol movement must survive the preceding state transition");
        }
    }
}

impl EngineInner {
    #[cfg(test)]
    pub(in crate::engine) fn execute_ai_coordinate_patrol(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        info: &crate::ai::StimulusInfo,
    ) {
        AiOwnerCtx::new(self, sim, assets, owner).execute_ai_coordinate_patrol(info)
    }

    /// Apply facing from the two actor values it actually reads. In particular,
    /// this runs after coordinate Think, so callback changes are visible.
    fn instruct_patrol_direction(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        member: EntityId,
        direction: u16,
    ) {
        let entity = self
            .entities()
            .expect_entity(member, format_args!("patrol direction member"));
        let current_direction = entity.element_data().direction() as u16;
        let action_state = entity
            .actor_data()
            .expect("patrol member has no actor data")
            .action_state;
        let ai = self.ai_mut(member, "patrol direction member");
        ai.patrol_direction = direction;
        if ai.current_substate == crate::ai::Substate::DefaultPatrolEnrouteWaiting {
            if direction == current_direction
                && matches!(
                    action_state,
                    crate::element::ActionState::Waiting | crate::element::ActionState::Bored
                )
            {
                ai.already_turned = true;
            } else {
                self.launch_live_ai_turn(sim, assets, member, direction as i16, false);
            }
        }
    }

    pub(in crate::engine) fn instruct_patrol_direction_to_patrol_members(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        owner: EntityId,
        assets: &LevelAssets,
        direction: u16,
    ) {
        let member_count = self.ai(owner, "patrol direction chief").patrol.len();
        for index in 0..member_count {
            let member = *self
                .ai(owner, "patrol direction chief")
                .patrol
                .get(index)
                .expect("patrol shrank during direction callback");
            self.instruct_patrol_direction(sim, assets, member, direction);
            // Register turns now; owner instruction belongs to the later
            // sequence-manager pass, as with coordinate Think below.
        }
    }

    /// Run one chief's patrol refresh. Only authored formation destinations and
    /// dispatch arguments cross callbacks; actors and obstacles are read from
    /// their owners, without an all-NPC patrol snapshot.
    pub(in crate::engine) fn tick_patrol_coordination_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        use crate::ai::{AiState, Stimulus, StimulusType, Substate};

        if self.actors_frozen() || self.is_very_very_busy(owner) {
            return;
        }
        let Some(ai) = self.entities().get(owner).and_then(Entity::ai_controller) else {
            return;
        };
        if !ai.needs_patrol_reinit && ai.patrol.is_empty() && ai.missed_patrol_members.is_empty() {
            return;
        }
        if ai.needs_patrol_reinit {
            self.initialize_patrol_for_npc(assets, owner);
        }

        let ai = self.ai(owner, "patrol chief");
        if (ai.patrol.is_empty() && ai.missed_patrol_members.is_empty())
            || ai.patrol_stopped
            || ai.current_state != AiState::Default
            || ai.current_substate == Substate::DefaultPatrolChiefReturnToPatrol
            || ai.patrol_path.is_none()
        {
            return;
        }

        let frame = self.control.frame_counter;
        let position = self.live_ai_position(owner);
        let entity = self.expect_entity(owner, "patrol chief");
        let direction = entity.element_data().direction() as u8;
        let bounds = *entity.position_iface().get_move_box();
        let bounds = if bounds.is_somewhere() {
            crate::coordinates::MoveBox::from_coords(
                bounds.x_min() - 3.0,
                bounds.y_min() - 3.0,
                bounds.x_max() + 3.0,
                bounds.y_max() + 3.0,
            )
        } else {
            crate::coordinates::MoveBox::new()
        };
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("patrol chief"));
        let path = ai
            .patrol_path
            .as_mut()
            .expect("validated patrol path disappeared");
        path.add_history_entry(position, direction);
        if frame & 7 != 0 {
            return;
        }

        // Formation geometry and loop extent are fixed before callbacks.
        // Membership and distance are read at each indexed call site.
        let positions =
            path.compute_patrol_positions(ai.patrol.len(), Some(&self.world.fast_grid), &bounds);
        for (index, (target, direction)) in positions.into_iter().enumerate() {
            let member = *self
                .ai(owner, "patrol chief")
                .patrol
                .get(index)
                .expect("patrol shrank during coordinate callback");
            let current = self.live_ai_position(member);
            if !((current.x - target.x)
                .abs()
                .max((current.y - target.y).abs())
                > 3.0)
            {
                continue;
            }
            let stimulus = Stimulus::with_position(StimulusType::CallPatrolCoordinate, target);
            self.debug_patrol_turn_lifecycle("before_coordinate_think", member);
            self.execute_ai_callback(sim, assets, member, &stimulus);
            // Construct Move before applying direction, but leave its deferred
            // InstructOwner for the normal sequence-manager phase.
            self.debug_patrol_turn_lifecycle("after_coordinate_think", member);
            let member = *self
                .ai(owner, "patrol chief after coordinate")
                .patrol
                .get(index)
                .expect("patrol shrank during coordinate callback");
            self.instruct_patrol_direction(sim, assets, member, direction);
            self.debug_patrol_turn_lifecycle("after_instructed_direction_emit", member);

            self.debug_patrol_turn_lifecycle("after_instructed_direction_drain", member);
        }
        self.reacquire_patrol_members(assets, owner);
    }

    fn reacquire_patrol_members(&mut self, assets: &LevelAssets, owner: EntityId) {
        let mut index = 0;
        while let Some(&member) = self
            .ai(owner, "patrol chief")
            .missed_patrol_members
            .get(index)
        {
            let entity = self.expect_entity(member, "missed patrol member");
            let npc = entity
                .ai_actor_data()
                .expect("missed patrol member has no AI actor data");
            let able_to_help = match entity {
                Entity::Soldier(soldier) => crate::ai_enemy::soldier_is_able_to_help_state(
                    !entity.is_dead() && !soldier.human.unconscious,
                    npc.ai_state(),
                    npc.ai_substate(),
                ),
                _ => false,
            };
            if missed_patrol_member_reacquired(
                true,
                || self.patrol_member_visible(assets, owner, member),
                able_to_help,
                npc.ai_state(),
            ) {
                let ai = self.ai_mut(owner, "patrol chief");
                ai.missed_patrol_members.remove(index);
                ai.patrol.push(member);
                self.ai_mut(member, "reacquired patrol member").patrol_chief = Some(owner);
            } else {
                index += 1;
            }
        }
    }
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_ai_coordinate_patrol(
        &mut self,
        info: &crate::ai::StimulusInfo,
    ) {
        use crate::ai::{AiState, GotoFlags, StimulusInfo, Substate};

        let ai = self.engine.ai(self.owner, "patrol coordinate owner");
        if ai.patrol_chief.is_none() {
            return;
        }
        let StimulusInfo::Position(target) = *info else {
            return;
        };
        match ai.current_substate {
            Substate::DefaultInMacro
            | Substate::DefaultEnroute
            | Substate::DefaultGotoPost
            | Substate::DefaultGotoPostTurn
            | Substate::DefaultOnPost
            | Substate::DefaultGotoChief
            | Substate::DefaultOnPostLookingSidewards => {
                self.stop_ai_owner();
            }
            Substate::DefaultPatrolEnroute
            | Substate::DefaultPatrolEnrouteRunning
            | Substate::DefaultPatrolEnrouteWaiting => {}
            _ => return,
        }

        let position = self.engine.live_ai_position(self.owner);
        let chief = self
            .engine
            .ai(self.owner, "patrol coordinate owner")
            .patrol_chief
            .expect("patrol chief disappeared during stop");
        let chief_position = self.engine.live_ai_position(chief);
        let to_point = [target.x - position.x, target.y - position.y];
        let to_chief = [chief_position.x - position.x, chief_position.y - position.y];
        let distance = (to_point[0] * to_point[0] + to_point[1] * to_point[1]).sqrt();
        let speed = crate::ai::PATROL_SPEED_BASE + distance / crate::ai::PATROL_SPEED_DIVISOR;
        let inverse_aspect = crate::position_interface::INVERSE_ASPECT_RATIO;
        if distance <= 30.0
            && to_chief[0] * to_point[0]
                + to_chief[1] * inverse_aspect * to_point[1] * inverse_aspect
                < 0.0
        {
            let direction = crate::position_interface::vector_to_sector_0_to_15(
                to_chief[0] * crate::position_interface::ASPECT_RATIO,
                to_chief[1],
            ) as u16;
            self.duty_face_direction(direction);
            return;
        }

        let walking = speed <= 2.0;
        let substate = if walking {
            Substate::DefaultPatrolEnroute
        } else {
            Substate::DefaultPatrolEnrouteRunning
        };
        self.duty_set_state(AiState::Default, substate);
        let (flags, speed) = if walking {
            let flags = self
                .engine
                .ai(self.owner, "patrol walking flags")
                .default_path_walking_flags;
            (GotoFlags::NO_HALT | GotoFlags::DONT_STOP | flags, speed)
        } else {
            (
                GotoFlags::RUN | GotoFlags::NO_HALT | GotoFlags::DONT_STOP,
                1.0,
            )
        };
        self.duty_go_to_speed(target, flags, speed);
    }
}
