//! Common duty execution with live actor reads and synchronous role changes.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::*;
    use crate::ai_enemy::{EnemyAi, task_priority};
    use crate::element::{Camp, DetectableType, Posture};
    use crate::engine::test_support::{
        actors::{make_test_ai_soldier, make_test_pc},
        square_sector,
    };

    fn fixture(count: usize) -> (EngineInner, LevelAssets, Vec<EntityId>) {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(128, 128);
        engine.world.fast_grid_mut().allocate_layers(1);
        let index = engine.world.fast_grid_mut().add_sector(
            square_sector(1, 0, MapPoint::new(0.0, 0.0), MapPoint::new(2000.0, 2000.0)),
            0,
        );
        let sector = crate::position_interface::SectorHandle::new(1)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap());
        let ids: Vec<EntityId> = (0..count)
            .map(|i| {
                let mut entity = make_test_ai_soldier(Camp::Lacklandists);
                let position = MapPoint::new(100.0 + i as f32 * 100.0, 100.0);
                entity.element_data_mut().set_position_map(position);
                entity.element_data_mut().set_sector(Some(sector));
                entity
                    .element_data_mut()
                    .set_position(crate::coordinates::WorldPoint3D::new(
                        position.x, position.y, 0.0,
                    ));
                entity.actor_data_mut().unwrap().action_state =
                    crate::element::ActionState::Waiting;
                entity.ai_actor_data_mut().unwrap().view_radius = 500;
                entity.npc_data_mut().unwrap().life_points = 100;
                let owner = engine.add_test_entity(entity);
                let ai = enemy_mut(&mut engine, owner);
                ai.base.owner_entity_id = Some(owner);
                ai.base.me = owner.index();
                ai.base.initial_position = Position {
                    x: 400.0,
                    y: 100.0,
                    sector: Some(sector),
                    level: 0,
                };
                owner
            })
            .collect();
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        if let Some(&owner) = ids.first() {
            engine.enter_ai_think_frame(owner);
        }
        (engine, assets, ids)
    }

    #[test]
    fn arrival_during_owner_notification_reads_the_removed_installed_order() {
        let (mut engine, assets, ids) = fixture(1);
        let owner = ids[0];
        let installed = engine.install_test_order(owner, crate::order::OrderType::WaitingUpright);
        engine
            .orders
            .sequence_manager
            .get_element_mut(
                installed.element.sequence_id,
                installed.element.element_index,
            )
            .unwrap()
            .orders
            .clear();
        let destination = engine.live_ai_position(owner);

        engine.duty_go_to_speed(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            destination,
            GotoFlags::empty(),
            1.0,
        );

        assert!(enemy(&engine, owner).base.already_on_point);
        assert_eq!(
            engine.actor_installed_order(owner).unwrap().order_type,
            crate::order::OrderType::WaitingUpright
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .get_element(
                    installed.element.sequence_id,
                    installed.element.element_index
                )
                .unwrap()
                .orders
                .is_empty()
        );
    }

    #[test]
    fn distant_destination_does_not_read_an_order_retired_during_owner_notification() {
        let (mut engine, assets, ids) = fixture(1);
        let owner = ids[0];
        let installed = engine.install_test_order(owner, crate::order::OrderType::WaitingUpright);
        let orders = &mut engine
            .orders
            .sequence_manager
            .get_element_mut(
                installed.element.sequence_id,
                installed.element.element_index,
            )
            .unwrap()
            .orders;
        // Deliberately invalidate this fixture's handle to test the guard's
        // short circuit independently of the normal installed-order lifetime.
        orders.release_slot(installed.slot);
        orders.clear();

        engine.duty_go_to_speed(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            Position {
                x: 500.0,
                y: 100.0,
                sector: None,
                level: 0,
            },
            GotoFlags::empty(),
            1.0,
        );

        assert!(enemy(&engine, owner).base.couldnt_reachpoint);
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .installed_order,
            Some(installed)
        );
    }

    #[test]
    fn facing_a_fractionally_elevated_target_registers_the_integral_direction() {
        let (mut engine, assets, ids) = fixture(1);
        let owner = ids[0];
        let entity = engine.get_entity_mut(owner).unwrap();
        entity
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(
                1027.3711, 439.47885, 0.0,
            ));
        entity
            .position_iface_mut()
            .set_direction(crate::position_interface::Direction::from_raw(13));
        let target = Position {
            x: 946.0,
            y: 401.0,
            ..engine.live_ai_position(owner)
        };

        engine.duty_face_position_at_elevation(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            target,
            7.790_524,
        );

        let turn = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| &sequence.elements)
            .find(|element| {
                element.owner == Some(owner) && element.command == crate::element::Command::Turn
            })
            .expect("integral target elevation must request a turn away from direction 13");
        assert!(matches!(
            turn.get_property(crate::sequence::Field::Direction),
            Some(crate::sequence::FieldValue::Integer(14)),
        ));
    }

    #[test]
    fn facing_elevation_sentinel_resolves_target_ground_height() {
        let (mut engine, assets, ids) = fixture(1);
        let owner = ids[0];
        let entity = engine.get_entity_mut(owner).unwrap();
        entity
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(1000.0, 500.0, 0.0));
        entity
            .position_iface_mut()
            .set_direction(crate::position_interface::Direction::from_raw(12));
        let target = Position {
            x: 920.0,
            y: 469.5,
            ..engine.live_ai_position(owner)
        };
        engine.duty_face_position_at_elevation(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            target,
            -1.0,
        );
        let turn = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| &sequence.elements)
            .find(|element| {
                element.owner == Some(owner) && element.command == crate::element::Command::Turn
            })
            .expect("ground-facing direction differs from the initial direction");
        assert!(matches!(
            turn.get_property(crate::sequence::Field::Direction),
            Some(crate::sequence::FieldValue::Integer(13)),
        ));
    }

    #[test]
    fn goto_flags_reach_registered_movement() {
        use crate::sequence::{MoveFlags, SequenceElementData};
        for (goto, expected) in [
            (GotoFlags::SWORD, MoveFlags::FORCE_SWORD_MOVEMENT),
            (GotoFlags::DONT_STOP, MoveFlags::NO_TRANSITIONS),
        ] {
            let (mut engine, assets, ids) = fixture(1);
            let owner = ids[0];
            let mut destination = engine.live_ai_position(owner);
            destination.x += 300.0;
            engine.duty_go_to(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                destination,
                goto,
            );
            assert!(engine.orders.sequence_manager.sequences_iter().any(|sequence|
                sequence.elements.iter().any(|element| element.owner == Some(owner)
                    && matches!(element.data, SequenceElementData::Movement { flags, .. } if flags.contains(expected)))));
        }
    }

    #[test]
    fn duty_clears_reciprocal_pc_guard() {
        let (mut engine, mut assets, ids) = fixture(1);
        let owner = ids[0];
        let pc = engine.add_test_entity(make_test_pc(Posture::Upright));
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        let EntityId::Pc(pc_id) = pc else {
            unreachable!()
        };
        let Entity::Pc(actor) = engine.world.entities.get_mut(pc).unwrap() else {
            unreachable!()
        };
        actor.pc.guard = Some(owner);
        let ai = enemy_mut(&mut engine, owner);
        ai.base.current_state = AiState::Menacing;
        ai.base.current_substate = Substate::MenacingPcInComa;
        ai.guarded_pc = Some(pc_id);
        duty(&mut engine, &assets, owner);
        assert_eq!(enemy(&engine, owner).guarded_pc, None);
        let Entity::Pc(actor) = engine.world.entities.get(pc).unwrap() else {
            unreachable!()
        };
        assert_eq!(actor.pc.guard, None);
    }

    #[test]
    fn duty_settles_attentive_exit_before_return_movement() {
        for attentive in [false, true] {
            let (mut engine, assets, ids) = fixture(1);
            let owner = ids[0];
            let ai = enemy_mut(&mut engine, owner);
            ai.attentive = attentive;
            ai.will_be_attentive = attentive;
            duty(&mut engine, &assets, owner);
            let ai = enemy(&engine, owner);
            assert_eq!(ai.attentive, attentive);
            assert!(!ai.will_be_attentive);
            assert_eq!(ai.base.last_goto_destination.x, 400.0);
            let commands: Vec<_> = engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|s| s.elements.iter())
                .filter(|e| e.owner == Some(owner))
                .map(|e| e.command)
                .collect();
            let movement = commands
                .iter()
                .position(|command| *command == crate::element::Command::Move)
                .expect("duty launches return movement");
            if attentive {
                let leave = commands
                    .iter()
                    .position(|command| *command == crate::element::Command::LeaveAttentiveMode)
                    .expect("attentive exit is registered");
                assert!(
                    leave < movement,
                    "attentive exit must be registered before movement"
                );
            }
        }
    }

    #[test]
    fn duty_chief_visibility_uses_body_but_approach_uses_selected_gate() {
        for visible_body in [false, true] {
            let (mut engine, assets, ids) = fixture(2);
            let (owner, chief) = (ids[0], ids[1]);
            engine.scripts.mission = Some(crate::engine::test_support::asm::empty_mission_script(
                "duty_gate.scs",
            ));
            let chief_x = if visible_body { 200.0 } else { 1200.0 };
            let endpoint = if visible_body { 1200.0 } else { 200.0 };
            let entity = engine.world.entities.get_mut(chief).unwrap();
            entity
                .element_data_mut()
                .set_position(crate::coordinates::WorldPoint3D::new(chief_x, 100.0, 0.0));
            entity
                .element_data_mut()
                .set_position_map(MapPoint::new(chief_x, 100.0));
            let sector = engine.live_ai_position(chief).sector.unwrap();
            engine
                .script_domains
                .interactables
                .doors
                .push(crate::gate::Door {
                    point_in: MapPoint::new(endpoint, 100.0),
                    point_out: MapPoint::new(endpoint, 100.0),
                    sector_in: crate::sector::SectorNumber::new(1),
                    sector_out: crate::sector::SectorNumber::new(1),
                    sector_in_index: sector.arena_index(),
                    sector_out_index: sector.arena_index(),
                    ..Default::default()
                });
            let mut pass = crate::sequence::SequenceElement::new_movement(
                1,
                crate::element::Command::PassDoor,
                Some(chief),
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
            engine.select_sequence_element(chief, Some((sequence, 0)));
            engine.element_in_progress(
                &crate::sim_rng::test_context(),
                &LevelAssets::new(),
                &mut Vec::new(),
                sequence,
                0,
            );
            enemy_mut(&mut engine, owner).base.patrol_chief = Some(chief);
            engine.execute_common_ai_duty(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                DutyFlags::empty(),
            );
            let ai = enemy(&engine, owner);
            if visible_body {
                assert_eq!(ai.base.current_substate, Substate::DefaultGotoChief);
                assert_eq!(ai.base.last_goto_destination.x, endpoint);
            } else {
                assert_ne!(ai.base.current_substate, Substate::DefaultGotoChief);
                assert_ne!(ai.base.last_goto_destination.x, endpoint);
            }
        }
    }

    #[test]
    fn remembered_ale_keeps_live_patrol_return_point() {
        let (mut engine, assets, ids) = fixture(2);
        let (owner, ale) = (ids[0], ids[1]);
        let here = engine.live_ai_position(owner);
        let destination = engine.live_ai_position(ale);
        let ai = enemy_mut(&mut engine, owner);
        ai.base.current_state = AiState::Wondering;
        ai.base.current_substate = Substate::WonderingDrinkingAle;
        ai.other_seen_ale.push(ale.index());
        duty(&mut engine, &assets, owner);
        let ai = enemy(&engine, owner);
        assert_eq!(ai.base.current_substate, Substate::WonderingApproachingAle);
        assert_eq!(
            ai.base.interesting_object,
            Some(AiEntityHandle::new(ale.index()))
        );
        assert_eq!(ai.return_to_patrol_point, here);
        assert_eq!(ai.base.last_goto_destination, destination);
    }

    #[test]
    fn one_point_path_keeps_initial_direction_conversion_and_virtual_duty() {
        let (mut engine, mut assets, ids) = fixture(1);
        let owner = ids[0];
        let here = engine.live_ai_position(owner);
        assets.navigation.hiking_paths =
            std::sync::Arc::new(vec![crate::level_data::RawHikingPath {
                waypoints: vec![crate::level_data::RawWaypoint {
                    x: here.x as i16,
                    y: here.y as i16,
                    sector: 1,
                    level: 0,
                    command: crate::level_data::WaypointCommand::None,
                }],
            }]);
        assets.navigation.hiking_waypoint_sectors =
            Some(std::sync::Arc::new(vec![vec![here.sector.unwrap()]]));
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(3);
        let ai = enemy_mut(&mut engine, owner);
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultGotoRouteTurn;
        ai.base.has_patrol_path = true;
        ai.base.patrol_path =
            PatrolPath::new(PathId::new(0).unwrap(), &assets.navigation.hiking_paths);
        ai.current_task_priority = task_priority::ENEMY;
        engine.execute_ai_callback(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &Stimulus::new(StimulusType::EventDone),
        );
        let ai = enemy(&engine, owner);
        assert!(!ai.base.has_patrol_path);
        assert_eq!(ai.base.initial_position, here);
        assert_eq!(ai.base.initial_view_direction, 2);
        assert_eq!(ai.current_task_priority, task_priority::NONE);
        assert_eq!(ai.base.current_substate, Substate::DefaultGotoPostTurn);
        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|s| s.elements.iter())
                .any(|e| e.owner == Some(owner) && e.command == crate::element::Command::Turn)
        );
    }

    #[test]
    fn reaching_officer_without_report_finishes_duty_on_engine_stack() {
        let (mut engine, assets, ids) = fixture(2);
        let (owner, officer) = (ids[0], ids[1]);
        let sim = crate::sim_rng::test_context();
        let ai = enemy_mut(&mut engine, owner);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingCharlyGoToOfficer;
        ai.base.antagonist = Some(AiEntityHandle::new(officer.index()));
        engine.execute_ai_officer_rpc(
            &sim,
            &assets,
            owner,
            &Stimulus::new(StimulusType::EventReachPoint),
        );
        let ai = enemy(&engine, owner);
        assert_eq!(ai.base.current_state, AiState::Default);
        assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
        assert_eq!(ai.base.antagonist, None);
    }

    #[test]
    fn special_strike_freeze_retains_event_and_unfreeze_completes_strike() {
        let (mut engine, assets, ids) = fixture(1);
        let owner = ids[0];
        let sim = crate::sim_rng::test_context();
        let ai = enemy_mut(&mut engine, owner);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfight;
        engine.control.frame_counter = 40;
        engine.begin_ai_special_strike(&sim, &assets, owner);
        engine.reconcile_ai_special_strike(&sim, &assets, owner, true);
        let ai = enemy(&engine, owner);
        assert!(ai.pending_special_strike);
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingSwordfightSpecialStrike
        );
        for locks in [
            AiLockFlags::BUSY,
            AiLockFlags::FREEZE,
            AiLockFlags::BUSY | AiLockFlags::FREEZE,
        ] {
            enemy_mut(&mut engine, owner).base.locks_flag_field = locks;
            engine.control.frame_counter = 41;
            engine.reconcile_ai_special_strike(&sim, &assets, owner, false);
            let ai = enemy(&engine, owner);
            assert!(ai.pending_special_strike);
            assert_eq!(
                ai.base.current_substate,
                Substate::AttackingSwordfightSpecialStrike
            );
        }
        enemy_mut(&mut engine, owner).base.locks_flag_field = AiLockFlags::FREEZE;
        engine.control.frame_counter = 41;
        engine.execute_ai_callback(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &Stimulus::new(StimulusType::EventDone),
        );
        engine.reconcile_ai_special_strike(&sim, &assets, owner, false);
        let ai = enemy_mut(&mut engine, owner);
        assert!(ai.pending_special_strike);
        assert_eq!(
            ai.base
                .stimulus_queue
                .iter()
                .map(|s| s.stimulus_type)
                .collect::<Vec<_>>(),
            vec![StimulusType::EventDone]
        );
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingSwordfightSpecialStrike
        );
        ai.base.non_script_unlock(AiLockFlags::FREEZE);
        let event = ai.base.stimulus_queue.remove(0);
        engine.control.frame_counter = 42;
        engine.execute_ai_callback(&crate::sim_rng::test_context(), &assets, owner, &event);
        let ai = enemy_mut(&mut engine, owner);
        assert!(!ai.pending_special_strike);
        assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
        assert_eq!(ai.next_sword_strike_frame, 62);
        engine.control.frame_counter = 62;
        engine.begin_ai_special_strike(&sim, &assets, owner);
        engine.duty_set_state(
            &sim,
            &assets,
            owner,
            AiState::Attacking,
            Substate::AttackingSwordfightParade,
        );
        engine.reconcile_ai_special_strike(&sim, &assets, owner, false);
        let ai = enemy(&engine, owner);
        assert!(!ai.pending_special_strike);
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingSwordfightParade
        );
    }

    #[test]
    fn swordfight_notification_does_not_launch_another_entry() {
        let (mut engine, assets, ids) = fixture(2);
        let (owner, target) = (ids[0], ids[1]);
        engine.execute_ai_callback(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &Stimulus::with_human(StimulusType::EventEnterSwordfight, target.index()),
        );
        let ai = enemy(&engine, owner);
        assert_eq!(
            ai.base.primary_target,
            Some(AiEntityHandle::new(target.index()))
        );
        assert_eq!(ai.base.current_state, AiState::Attacking);
        assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
        assert!(
            !engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|s| s.elements.iter())
                .any(|e| e.owner == Some(owner)
                    && e.command == crate::element::Command::EnterSwordfight)
        );
    }

    #[test]
    fn attentive_forgetting_preserves_script_latch_through_live_callbacks() {
        for stimulus in [
            StimulusType::EventLoseConsciousness,
            StimulusType::EventWasp,
            StimulusType::EventNet,
        ] {
            for forced in [false, true] {
                let (mut engine, assets, ids) = fixture(1);
                let owner = ids[0];
                let ai = enemy_mut(&mut engine, owner);
                ai.attentive = true;
                ai.will_be_attentive = true;
                ai.forced_attentive = forced;
                engine.execute_ai_callback(
                    &crate::sim_rng::test_context(),
                    &assets,
                    owner,
                    &Stimulus::new(stimulus),
                );
                let ai = enemy(&engine, owner);
                assert!(!ai.attentive, "{stimulus:?}");
                assert!(!ai.will_be_attentive, "{stimulus:?}");
                assert_eq!(ai.forced_attentive, forced);
                if stimulus == StimulusType::EventLoseConsciousness {
                    engine.execute_ai_callback(
                        &crate::sim_rng::test_context(),
                        &assets,
                        owner,
                        &Stimulus::new(StimulusType::EventFitAgain),
                    );
                    let ai = enemy(&engine, owner);
                    assert_eq!(ai.base.current_substate, Substate::SleepingAwakening);
                    assert_eq!(ai.forced_attentive, forced);
                }
            }
        }
    }

    #[test]
    fn money_watch_skips_looted_body_and_marks_next_live_body() {
        let (mut engine, assets, ids) = fixture(3);
        let (owner, looted, unlooted) = (ids[0], ids[1], ids[2]);
        for body in [looted, unlooted] {
            let entity = engine.world.entities.get_mut(body).unwrap();
            entity.human_data_mut().unwrap().unconscious = true;
            let ai = entity.enemy_ai_mut().unwrap();
            ai.base.knocked_out_in_money_fight = true;
            ai.base.looted_after_money_fight = body == looted;
        }
        let ai = enemy_mut(&mut engine, owner);
        ai.base.current_state = AiState::Wondering;
        ai.base.current_substate = Substate::WonderingWatchingForMoreMoney;
        engine.execute_ai_callback(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &Stimulus::new(StimulusType::EventDone),
        );
        let ai = enemy(&engine, owner);
        assert_eq!(
            ai.base.detected_body,
            Some(AiEntityHandle::new(unlooted.index()))
        );
        assert_eq!(
            ai.base.current_substate,
            Substate::WonderingApproachingToLoot
        );
        assert!(enemy(&engine, unlooted).base.looted_after_money_fight);
    }

    fn enemy(engine: &EngineInner, owner: EntityId) -> &EnemyAi {
        engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .enemy_ai()
            .unwrap()
    }
    fn enemy_mut(engine: &mut EngineInner, owner: EntityId) -> &mut EnemyAi {
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .enemy_ai_mut()
            .unwrap()
    }
    fn duty(engine: &mut EngineInner, assets: &LevelAssets, owner: EntityId) {
        engine.execute_ai_return_to_duty(
            &crate::sim_rng::test_context(),
            assets,
            owner,
            DutyFlags::empty(),
        );
    }

    #[test]
    fn duty_resets_state_priority_and_finishes_movement() {
        let (mut engine, assets, ids) = fixture(1);
        let owner = ids[0];
        let ai = enemy_mut(&mut engine, owner);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfight;
        ai.current_task_priority = task_priority::ENEMY;
        duty(&mut engine, &assets, owner);
        let ai = enemy(&engine, owner);
        assert_eq!(ai.base.current_state, AiState::Default);
        assert_eq!(ai.current_task_priority, task_priority::NONE);
        assert!(!ai.base.needs_patrol_reinit);
        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|s| s.elements.iter())
                .any(|element| element.owner == Some(owner)
                    && element.command == crate::element::Command::Move)
        );
    }

    #[test]
    fn duty_releases_actual_archery_reservations() {
        let (mut engine, assets, ids) = fixture(1);
        let owner = ids[0];
        let position = engine.live_ai_position(owner);
        engine.ai.global.archery_sectors.push(SectorArchery {
            points: vec![PointArchery {
                position,
                direction: 0,
                is_shooting_point: true,
                sector_index: crate::sector::SectorNumber::new(1),
                owner: Some(owner),
            }],
            polygon: vec![],
            layer: 0,
            index_first_shooting_point: Some(crate::sector::ArcheryPointIdx(0)),
            index_last_shooting_point: Some(crate::sector::ArcheryPointIdx(0)),
            num_shooting_points: 1,
            num_owners: 1,
        });
        let ai = enemy_mut(&mut engine, owner);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingArcherWaitOnArcheryPath;
        ai.my_archery_sector = Some(0);
        ai.my_shooting_point = Some((0, 0));
        duty(&mut engine, &assets, owner);
        assert_eq!(enemy(&engine, owner).my_shooting_point, None);
        assert_eq!(enemy(&engine, owner).my_archery_sector, None);
        assert_eq!(engine.ai.global.archery_sectors[0].num_owners, 0);
        assert_eq!(engine.ai.global.archery_sectors[0].points[0].owner, None);
    }

    #[test]
    fn duty_deletes_beggars_added_before_call_and_retains_enemies() {
        let (mut engine, assets, ids) = fixture(2);
        let (owner, target) = (ids[0], ids[1]);
        let Entity::Soldier(soldier) = engine.world.entities.get_mut(target).unwrap() else {
            unreachable!()
        };
        soldier.soldier.cached_camp = Camp::Royalists;
        engine.execute_ai_add_detectable(owner, target, DetectableType::Beggar);
        engine.execute_ai_add_detectable(owner, target, DetectableType::Enemy);
        duty(&mut engine, &assets, owner);
        let npc = engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .ai_actor_data()
            .unwrap();
        assert!(npc.detectable_lists[DetectableType::Beggar as usize].is_empty());
        assert!(
            npc.detectable_lists[DetectableType::Enemy as usize]
                .iter()
                .any(|d| d.element == Some(target))
        );
    }

    #[test]
    fn duty_state_timer_reset_precedes_new_bored_timer() {
        for sitting in [false, true] {
            let (mut engine, assets, ids) = fixture(1);
            let owner = ids[0];
            engine.control.frame_counter = 254;
            if sitting {
                engine
                    .world
                    .entities
                    .get_mut(owner)
                    .unwrap()
                    .set_posture(Posture::Sitting);
            }
            let here = engine.live_ai_position(owner);
            let ai = enemy_mut(&mut engine, owner);
            ai.base.timer_is_running = true;
            ai.base.when_does_timer_ring = 999;
            ai.base.likes_to_sit_around = sitting;
            if sitting {
                ai.base.initial_position = here;
            }
            duty(&mut engine, &assets, owner);
            let ai = enemy(&engine, owner);
            assert_eq!(ai.base.timer_is_running, sitting);
            if sitting {
                assert_eq!(ai.base.current_substate, Substate::DefaultOnPost);
                assert!((324..394).contains(&ai.base.when_does_timer_ring));
            } else {
                assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
            }
        }
    }

    #[test]
    fn state_tail_reads_callback_mutations_but_keeps_entry_forced_attentive() {
        let (mut engine, assets, ids) = fixture(1);
        let owner = ids[0];
        let ai = enemy_mut(&mut engine, owner);
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultOnPost;
        ai.forced_attentive = false;
        let forced = engine.begin_live_enemy_state(
            &assets,
            owner,
            AiState::Default,
            Substate::DefaultInMacro,
        );
        let ai = enemy_mut(&mut engine, owner);
        ai.forced_attentive = true;
        ai.base.current_state = AiState::Sleeping;
        engine
            .get_entity_mut(owner)
            .unwrap()
            .ai_actor_data_mut()
            .unwrap()
            .eye_status = crate::element::EyeStatus::Closed;
        engine.finish_live_enemy_state(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            AiState::Default,
            Substate::DefaultInMacro,
            forced,
        );
        let ai = enemy(&engine, owner);
        assert!(!ai.will_be_attentive);
        assert_eq!(ai.base.current_substate, Substate::DefaultInMacro);
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .ai_actor_data()
                .unwrap()
                .eye_status,
            crate::element::EyeStatus::LookForward
        );
    }

    #[test]
    fn under_net_state_tail_restores_green_alert() {
        let (mut engine, assets, ids) = fixture(1);
        let owner = ids[0];
        engine.execute_ai_set_alert_status(&assets, owner, AlertLevel::Red, AlertFlags::empty());
        engine.duty_set_state(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            AiState::Wondering,
            Substate::WonderingUnderNet,
        );
        assert_eq!(
            enemy(&engine, owner).base.view_alert_status,
            AlertLevel::Green
        );
    }

    #[test]
    fn deep_duty_keeps_close_point_on_live_recursion_stack() {
        let (mut engine, assets, ids) = fixture(1);
        let owner = ids[0];
        let here = engine.live_ai_position(owner);
        let ai = enemy_mut(&mut engine, owner);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontime;
        ai.base.initial_position = here;
        for _ in 1..100 {
            engine.enter_ai_think_frame(owner);
        }
        duty(&mut engine, &assets, owner);
        let ai = enemy(&engine, owner);
        assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
        assert!(ai.base.already_on_point);
        assert_eq!(engine.ai_think_depth(), 100);
    }

    #[test]
    fn duty_clears_reciprocal_combat_neighbours() {
        let (mut engine, assets, ids) = fixture(3);
        let (owner, left, right) = (ids[0], ids[1], ids[2]);
        enemy_mut(&mut engine, owner).left_combat_neighbour =
            Some(AiEntityHandle::new(left.index()));
        enemy_mut(&mut engine, owner).right_combat_neighbour =
            Some(AiEntityHandle::new(right.index()));
        enemy_mut(&mut engine, left).right_combat_neighbour =
            Some(AiEntityHandle::new(owner.index()));
        enemy_mut(&mut engine, right).left_combat_neighbour =
            Some(AiEntityHandle::new(owner.index()));
        duty(&mut engine, &assets, owner);
        assert_eq!(enemy(&engine, owner).left_combat_neighbour, None);
        assert_eq!(enemy(&engine, owner).right_combat_neighbour, None);
        assert_eq!(enemy(&engine, left).right_combat_neighbour, None);
        assert_eq!(enemy(&engine, right).left_combat_neighbour, None);
    }

    #[test]
    fn duty_clears_reciprocal_shield_pair_from_either_side() {
        for returning_archer in [false, true] {
            let (mut engine, mut assets, ids) = fixture(3);
            let (archer, bearer) = (ids[0], ids[1]);
            std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].shield = true;
            let entity = engine.get_entity_mut(bearer).unwrap();
            let mut conversion = (*entity.element_data().sprite.conversion).clone();
            conversion.resize(
                conversion
                    .len()
                    .max(crate::order::OrderType::WaitingShield as usize + 1),
                u16::MAX,
            );
            conversion[crate::order::OrderType::WaitingShield as usize] = 0;
            entity.element_data_mut().sprite.conversion = std::sync::Arc::new(conversion);
            let ai = enemy_mut(&mut engine, archer);
            ai.is_archer_unit = true;
            ai.base.current_state = AiState::Attacking;
            ai.base.current_substate = Substate::AttackingBowShooting;
            ai.shield_bearer_before_me = Some(AiEntityHandle::new(bearer.index()));
            let ai = enemy_mut(&mut engine, bearer);
            ai.base.current_state = AiState::Attacking;
            ai.base.current_substate = Substate::AttackingProtectingWithShield;
            ai.base.primary_target = Some(AiEntityHandle::new(ids[2].index()));
            ai.archer_behind_me = Some(AiEntityHandle::new(archer.index()));
            duty(
                &mut engine,
                &assets,
                if returning_archer { archer } else { bearer },
            );
            assert_eq!(enemy(&engine, archer).shield_bearer_before_me, None);
            assert_eq!(enemy(&engine, bearer).archer_behind_me, None);
            if returning_archer {
                engine
                    .world
                    .entities
                    .get_mut(bearer)
                    .unwrap()
                    .actor_data_mut()
                    .unwrap()
                    .action_state = crate::element::ActionState::HoldingShield;
                enemy_mut(&mut engine, bearer).base.launch_timer(0, 0);
                engine.execute_ai_callback(
                    &crate::sim_rng::test_context(),
                    &assets,
                    bearer,
                    &Stimulus::new(StimulusType::EventTimer),
                );
                assert_eq!(
                    enemy(&engine, bearer).base.current_substate,
                    Substate::AttackingOverviewLookLeft
                );
            }
        }
    }
}

use super::*;
use crate::ai::{
    AiState, AlertFlags, AlertLevel, DutyFlags, GotoFlags, Position, Substate, WillStopCaller,
};

impl EngineInner {
    pub(in crate::engine) fn duty_set_state(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        state: AiState,
        substate: Substate,
    ) {
        let forced_attentive = if self
            .expect_entity(owner, "state-change owner")
            .enemy_ai()
            .is_some()
        {
            Some(self.begin_live_enemy_state(assets, owner, state, substate))
        } else {
            let entity = self
                .world
                .entities
                .expect_entity_mut(owner, format_args!("state-change owner"));
            let ai = entity
                .friendly_ai_mut()
                .expect("state-change owner has no role");
            debug_assert_eq!(substate.ai_state_family(), Some(state));
            ai.base
                .register_log_line(crate::ai::LogLineType::ChangeState, substate as u16);
            let alert = match state {
                AiState::Sleeping | AiState::Default | AiState::Wondering => {
                    crate::ai::AlertLevel::Green
                }
                AiState::Seeking | AiState::Fleeing => crate::ai::AlertLevel::Yellow,
                _ => panic!("Civilian AI entered invalid state: {state:?}"),
            };
            self.execute_ai_set_alert_status(assets, owner, alert, crate::ai::AlertFlags::empty());
            None
        };

        let ai = self.ai(owner, "state-change owner");
        let notify = forced_attentive.is_none() || ai.current_substate != substate;
        let source = match state {
            AiState::Attacking | AiState::Menacing | AiState::Fleeing => {
                crate::ai::AiStateChangeSource::from_optional_human(ai.primary_target)
            }
            _ => crate::ai::AiStateChangeSource::SelfActor,
        };
        if notify {
            self.call_live_ai_state_change_filter(sim, assets, owner, state, source);
        }
        if let Some(forced_attentive) = forced_attentive {
            self.finish_live_enemy_state(sim, assets, owner, state, substate, forced_attentive);
        } else {
            let entity = self
                .world
                .entities
                .expect_entity_mut(owner, format_args!("state-change callback owner"));
            let ai = entity
                .ai_controller_mut()
                .expect("state-change callback removed owner AI");
            ai.set_ai_state(state);
            ai.current_substate = substate;
        }
    }

    pub(in crate::engine) fn duty_face_direction(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        direction: u16,
    ) {
        let entity = self
            .world
            .entities
            .expect_entity_mut(owner, format_args!("duty facing owner"));
        let current_direction = entity.element_data().direction() as u16;
        let action_state = entity
            .actor_data()
            .expect("facing requires actor")
            .action_state;
        if current_direction == direction
            && matches!(
                action_state,
                crate::element::ActionState::Waiting | crate::element::ActionState::Bored
            )
        {
            entity
                .ai_controller_mut()
                .expect("facing requires controller")
                .already_turned = true;
        } else {
            self.launch_live_ai_turn(sim, assets, owner, direction as i16, false);
        }
    }

    pub(in crate::engine) fn duty_face_position_at_elevation(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        position: Position,
        elevation: f32,
    ) {
        // Facing accepts a signed integral elevation, including the -1
        // sentinel for resolving the target's ground point.
        let elevation = elevation as i16;
        let entity = self.expect_entity(owner, "duty facing position owner");
        let (dx, dy) = if elevation == -1 {
            let target = self.position_to_point_3d(
                assets,
                position.sector,
                position.level,
                position.x,
                position.y,
            );
            let here = entity.element_data().position();
            (target.x - here.x, target.y - here.y)
        } else {
            let here = self.live_ai_position(owner);
            (
                position.x - here.x,
                (position.y - here.y)
                    + (f32::from(elevation) - entity.position_iface().get_elevation()),
            )
        };
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy);
        self.duty_face_direction(sim, assets, owner, direction as u16);
    }

    pub(in crate::engine) fn duty_point_to(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        position: Position,
    ) {
        let target = self.position_to_point_3d(
            assets,
            position.sector,
            position.level,
            position.x,
            position.y,
        );
        let body = self
            .expect_entity(owner, "pointing owner")
            .element_data()
            .position();
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
            target.x - body.x,
            target.y - body.y,
        );
        use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};
        let mut turn = SequenceElement::new_generic(1, crate::element::Command::Turn, Some(owner));
        turn.set_property(Field::Direction, FieldValue::Integer(direction as u32));
        let mut point =
            SequenceElement::new_generic(2, crate::element::Command::Point, Some(owner));
        point.set_property(Field::Direction, FieldValue::Integer(direction as u32));
        let mut sequence = Sequence::new();
        sequence.append_element(turn);
        sequence.append_element(point);
        self.launch_sequence(sim, assets, sequence);
    }

    pub(in crate::engine) fn duty_go_to(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        destination: Position,
        flags: GotoFlags,
    ) {
        self.duty_go_to_speed(sim, assets, owner, destination, flags, 1.0);
    }

    pub(in crate::engine) fn duty_go_to_speed(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        destination: Position,
        flags: GotoFlags,
        speed: f32,
    ) {
        self.ai_mut(owner, "movement request owner")
            .begin_move_request(destination, flags);
        let mut destination = destination;
        if flags.contains(GotoFlags::FIND_ACCESSIBLE)
            && !self.resolve_ai_accessible_destination(owner, &mut destination)
        {
            return;
        }
        let position = self.live_ai_position(owner);
        let depth = self.ai_think_depth();
        let entity = self
            .world
            .entities
            .expect_entity_mut(owner, format_args!("duty movement owner"));
        let element = entity.element_data();
        let layer = element.layer();
        let sector = element.sector();
        let actor = entity.actor_data().expect("duty movement requires actor");
        let installed_order = actor.installed_order;
        let civilian = entity.is_civilian();
        let ai = entity
            .ai_controller_mut()
            .expect("duty movement requires controller");
        let flags = {
            let mut flags = flags;
            if civilian {
                flags -= GotoFlags::FORBIDDEN_CIVILIANS;
            }
            let dx = (position.x - destination.x).abs();
            let dy = (position.y - destination.y).abs();
            if dx.max(dy) < 5.0
                && !ai.likes_to_sit_around
                && !ai.special_action
                && matches!(
                    installed_order
                        .map(|handle| handle.resolve(&self.orders.sequence_manager).order_type)
                        .unwrap_or(crate::order::OrderType::NonanimationEnd),
                    crate::order::OrderType::WaitingUpright
                        | crate::order::OrderType::WaitingAlerted
                        | crate::order::OrderType::NonanimationEnd
                )
            {
                if depth > 0 {
                    ai.already_on_point = true;
                } else {
                    self.execute_ai_callback(
                        sim,
                        assets,
                        owner,
                        &crate::ai::Stimulus::new(crate::ai::StimulusType::EventReachPoint),
                    );
                }
                return;
            }
            let tolerance = if flags.contains(GotoFlags::NEAR) {
                ai.stop_before_end_of_path_distance as f32
            } else {
                0.0
            };
            if flags.contains(GotoFlags::NEAR)
                && destination.level == layer
                && dx * dx + dy * dy <= tolerance * tolerance
            {
                if depth > 0 {
                    ai.already_on_point = true;
                } else {
                    self.execute_ai_callback(
                        sim,
                        assets,
                        owner,
                        &crate::ai::Stimulus::new(crate::ai::StimulusType::EventReachPoint),
                    );
                }
                return;
            }
            if destination.x <= 0.0
                || destination.y <= 0.0
                || destination.sector.is_none()
                || (destination.level as i16) < 0
            {
                ai.couldnt_reachpoint = true;
                return;
            }
            let crosses_sector = match (
                destination.sector.and_then(|s| s.arena_index()),
                sector.and_then(|s| s.arena_index()),
            ) {
                (Some(destination), Some(current)) => destination != current,
                _ => destination.sector != sector,
            };
            if flags.contains(GotoFlags::STRAIGHT)
                && !flags.contains(GotoFlags::ASK_OBSTACLE)
                && (crosses_sector || destination.level != layer)
            {
                flags -= GotoFlags::STRAIGHT;
            }
            flags
        };
        if !self.authorize_ai_destination(
            owner,
            destination,
            true,
            flags.contains(GotoFlags::ASK_OBSTACLE),
        ) {
            return;
        }
        self.launch_ai_move(sim, assets, owner, destination, flags, speed);
    }

    pub(in crate::engine) fn duty_go_near(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        destination: Position,
        distance: i32,
        flags: GotoFlags,
    ) {
        let depth = self.ai_think_depth();
        self.ai_mut(owner, "duty approach owner")
            .prepare_approach(distance, flags, depth);
        self.duty_go_to(sim, assets, owner, destination, flags | GotoFlags::NEAR);
    }

    pub(in crate::engine) fn execute_common_ai_duty(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        flags: DutyFlags,
    ) {
        self.execute_ai_set_alert_status(assets, owner, AlertLevel::Green, AlertFlags::empty());
        {
            let entity = self
                .world
                .entities
                .expect_entity_mut(owner, format_args!("common duty owner"));
            let no_friends = entity
                .ai_actor_data()
                .expect("duty owner has no actor")
                .detectable_lists[crate::element::DetectableType::Friend as usize]
                .is_empty();
            let ai = entity
                .ai_controller_mut()
                .expect("duty owner has no controller");
            if !flags.contains(DutyFlags::KEEP_EMOTICON) {
                ai.clear_emoticon();
            }
            if let Some(path) = &mut ai.patrol_path {
                path.reset_history();
            } else {
                ai.detached_patrol_path_status.history.clear();
            }
            ai.my_reconnaissance_report.reset();
            if no_friends {
                ai.detected_body = None;
            }
        }

        let chief = self.ai(owner, "duty chief query").patrol_chief;
        if let Some(chief) = chief {
            let able = match self.expect_entity(chief, "duty patrol chief") {
                Entity::Soldier(soldier) => crate::element::Human::is_able_to_fight(soldier),
                Entity::Pc(pc) => crate::element::Human::is_able_to_fight(pc),
                _ => false,
            };
            if able && self.patrol_member_visible(assets, owner, chief) {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Default,
                    Substate::DefaultGotoChief,
                );
                // State notifications may replace the chief before the approach.
                let chief = self
                    .ai(owner, "duty chief after state")
                    .patrol_chief
                    .expect("duty state callback cleared required patrol chief");
                let destination = self.live_ai_position(chief);
                self.duty_go_near(
                    sim,
                    assets,
                    owner,
                    destination,
                    crate::parameters_ai::AI_TALK_DISTANCE,
                    GotoFlags::empty(),
                );
                let ai = self.ai_mut(owner, "duty approach result");
                if !ai.couldnt_reachpoint {
                    return;
                }
                ai.couldnt_reachpoint = false;
            }
        }

        if self.ai(owner, "duty path query").has_patrol_path {
            let here = self.live_ai_position(owner);
            let paths = &assets.navigation.hiking_paths;
            let nearest_distance = {
                let ai = self.ai_mut(owner, "duty path selection");
                let path = ai
                    .patrol_path
                    .as_mut()
                    .expect("duty path flag requires initialized path");
                let first = path
                    .get_waypoint(0, paths)
                    .expect("duty path requires a waypoint");
                let mut best = 0;
                let mut distance = (first.x as f32 - here.x)
                    .abs()
                    .max((first.y as f32 - here.y).abs());
                for index in 0..path.size {
                    let waypoint = path.get_waypoint(index, paths).expect("duty path waypoint");
                    let candidate = (waypoint.x as f32 - here.x)
                        .abs()
                        .max((waypoint.y as f32 - here.y).abs());
                    if candidate < distance {
                        best = index;
                        distance = candidate;
                    }
                }
                path.set_current_index(best);
                let waypoint = path
                    .current_waypoint(paths)
                    .expect("selected duty waypoint");
                let dx = waypoint.x as f32 - here.x;
                let dy = waypoint.y as f32 - here.y;
                let nearest_distance = dx.abs().max(dy.abs());
                if best < path.size - 1 {
                    let next = path.peek_next_waypoint(paths).expect("next duty waypoint");
                    if nearest_distance < 10.0
                        || dx * (next.x as f32 - waypoint.x as f32)
                            + dy * (next.y as f32 - waypoint.y as f32)
                            < 0.0
                    {
                        path.advance();
                    }
                }
                nearest_distance
            };
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Default,
                Substate::DefaultGotoRoute,
            );
            let frame = self.control.frame_counter;
            let creation_order = self.world.original_creation_order(owner);
            {
                let ai = self.ai_mut(owner, "duty route history");
                if ai.has_patrol() && frame == 0 && nearest_distance < 50.0 {
                    ai.patrol_path
                        .as_mut()
                        .expect("duty route state requires path")
                        .initialize_history_entries_on_path(paths, |p, w, s| {
                            assets.navigation.hiking_waypoint_sector(p, w, s)
                        });
                }
            }
            let walk_flags = {
                let ai = self.ai_mut(owner, "duty route forecast");
                let stop = ai.will_stop_at_next_waypoint_at(
                    sim,
                    paths,
                    frame,
                    Some(creation_order),
                    WillStopCaller::ReturnToDuty,
                );
                ai.default_path_walking_flags
                    | if stop {
                        GotoFlags::empty()
                    } else {
                        GotoFlags::DONT_STOP
                    }
            };
            let destination = {
                let path = self
                    .ai(owner, "duty route destination")
                    .patrol_path
                    .as_ref()
                    .expect("duty route requires path");
                let waypoint = path
                    .current_waypoint(paths)
                    .expect("duty route requires waypoint");
                Position {
                    x: waypoint.x as f32,
                    y: waypoint.y as f32,
                    sector: assets.navigation.hiking_waypoint_sector(
                        usize::from(path.hiking_path_index),
                        usize::from(path.current_waypoint_index),
                        waypoint.sector,
                    ),
                    level: waypoint.level,
                }
            };
            self.duty_go_to(sim, assets, owner, destination, walk_flags);
            return;
        }

        let (posture, initial, special_posture) = {
            let entity = self.expect_entity(owner, "duty post owner");
            let ai = entity.ai_controller().expect("duty post controller");
            let special = if ai.likes_to_sit_around {
                Some(crate::element::Posture::Sitting)
            } else if ai.special_action {
                Some(crate::element::Posture::Leisure)
            } else {
                None
            };
            (
                entity.element_data().posture(),
                ai.initial_position,
                special,
            )
        };
        let here = self.live_ai_position(owner);
        if special_posture == Some(posture)
            && (here.x - initial.x).abs().max((here.y - initial.y).abs()) < 3.0
        {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Default,
                Substate::DefaultOnPost,
            );
            let bored = self.ai_bored_time(sim, assets, owner);
            let frame = self.control.frame_counter;
            let ai = self.ai_mut(owner, "duty post timer");
            ai.launch_timer(bored as u32, frame);
        } else {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Default,
                Substate::DefaultGotoPost,
            );
            let initial = self.ai(owner, "duty post after state").initial_position;
            self.duty_go_to(
                sim,
                assets,
                owner,
                initial,
                if special_posture.is_some() {
                    GotoFlags::SPECIAL_ACTION
                } else {
                    GotoFlags::empty()
                },
            );
        }
    }
}
