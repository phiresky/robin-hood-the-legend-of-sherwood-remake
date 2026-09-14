use super::*;
use crate::coordinates::{MapPoint, WorldPoint3D};
use crate::element::Command;
use crate::order::OrderType;

#[test]
fn after_combat_injury_speaks_once_only_after_a_rejected_strike_proposal() {
    use crate::ai::{LogLineType, Remark, Stimulus};
    use crate::element::ActionState;
    use crate::engine::SimulationRng;
    use crate::sim_rng::RngSite;

    for (roll, accepted) in [(85, false), (0, true)] {
        let (mut engine, mut assets, owner, target) = fixture(Substate::AttackingSwordfight);
        place(&mut engine, target, 110.0, 100.0, 0.0);
        for (id, opponent, dx) in [(owner, target, 10.0), (target, owner, -10.0)] {
            let entity = engine.get_entity_mut(id).unwrap();
            entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
            entity.human_data_mut().unwrap().opponents.clear();
            entity.human_data_mut().unwrap().opponents.push(opponent);
            entity.element_data_mut().set_direction_instantly(
                crate::position_interface::vector_to_sector_0_to_15_iso(dx, 0.0) as i16,
            );
        }
        let initial = {
            let entity = engine.get_entity(owner).unwrap();
            crate::ai::Position {
                x: 100.0,
                y: 100.0,
                sector: entity.element_data().sector(),
                level: entity.element_data().layer(),
            }
        };
        let ai = engine.combat_event_ai_mut(owner);
        ai.combat_trainer = true;
        ai.base.initial_position = initial;
        ai.base.ai_log.clear();
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles.soldiers[0].fighting = 20;
        profiles.soldiers[0].exclamation_id = 221;
        profiles.soldiers[0].hth_weapon_id = 1;
        profiles.characters[0].hth_weapon_id = 1;
        let mut weapon = crate::profiles::HtHWeaponProfile::default();
        weapon.distance[crate::weapons::WeaponDistance::Maximal as usize] = 30;
        let thrust = &mut weapon.thrusts[crate::weapons::SwordStrike::A as usize];
        thrust.energy = 7;
        thrust.maximal_distance = 30;
        thrust.cutting = 4;
        profiles.hth_weapons[0] = weapon;

        engine.control.rng = SimulationRng::with_original_replay(vec![roll, 37]);
        engine.with_simulation_context(|engine, sim| {
            engine.execute_ai_callback(
                sim,
                &assets,
                owner,
                &Stimulus::new(StimulusType::EventAfterCombatInjury),
            );
        });

        let ai = engine.combat_event_ai(owner);
        assert_eq!(
            ai.base.current_substate,
            if accepted {
                Substate::AttackingSwordfightSpecialStrike
            } else {
                Substate::AttackingSwordfight
            }
        );
        assert_eq!(
            ai.base
                .ai_log
                .iter()
                .filter(|line| {
                    line.line_type == LogLineType::Speak && line.info == Remark::CombatInsult as u16
                })
                .count(),
            usize::from(!accepted)
        );
        assert_eq!(
            engine
                .feedback
                .sound_sim
                .pending_exclamations
                .iter()
                .filter(|speech| {
                    speech.actor_id == owner.index()
                        && speech.exclamation_id == Remark::CombatInsult as u16
                })
                .count(),
            usize::from(!accepted)
        );
        assert_eq!(engine.control.rng.original_replay_cursor(), Some(1));
        assert_eq!(
            engine.control.rng.original_replay_sites(0..1).unwrap(),
            vec![RngSite::SwordStrikeSelection]
        );
        engine.with_simulation_context(|_, sim| {
            assert_eq!(crate::sim_rng::u32(sim, RngSite::ScriptRand, 0..100), 37);
        });
    }
}

#[test]
fn enemy_near_retargets_only_the_four_observing_substates() {
    for (substate, enters) in [
        (Substate::AttackingReactiontimeTurning, true),
        (Substate::AttackingReactiontime, true),
        (Substate::AttackingApproachToObserve, true),
        (Substate::AttackingObserve, true),
        (Substate::AttackingRunningToEnemy, false),
    ] {
        let (mut engine, assets, owner, target) = fixture(substate);
        place(&mut engine, target, 120.0, 100.0, 0.0);
        let ai = engine.combat_event_ai_mut(owner);
        ai.base.primary_target = None;
        ai.combat_trainer = true;
        let handled = engine.execute_ai_combat_unexpected_event(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &crate::ai::Stimulus::with_human(StimulusType::EventEnemyNear, target.index()),
        );
        assert!(handled);
        let ai = engine.combat_event_ai(owner);
        assert_eq!(
            ai.base.primary_target,
            enters.then_some(crate::ai::AiEntityHandle::new(target.index()))
        );
        assert_eq!(
            ai.base.current_substate,
            if enters {
                Substate::AttackingSwordfight
            } else {
                substate
            }
        );
        if enters {
            assert_eq!(ai.base.when_does_timer_ring, 120);
            assert!(engine.orders.sequence_manager.sequences_iter().flat_map(|sequence| sequence.elements.iter()).any(|element| {
                element.owner == Some(owner) && element.command == Command::EnterSwordfight
                    && matches!(element.get_property(crate::sequence::Field::Opponent), Some(crate::sequence::FieldValue::Element(opponent)) if *opponent == target)
            }));
        }
    }
}

#[test]
fn live_swordfight_entry_clears_reciprocal_neighbours() {
    let (mut engine, assets, owner, target) = fixture(Substate::AttackingObserve);
    place(&mut engine, target, 120.0, 100.0, 0.0);
    let left = engine.add_test_entity(crate::engine::test_support::actors::make_test_ai_soldier(
        Camp::Lacklandists,
    ));
    let right = engine.add_test_entity(crate::engine::test_support::actors::make_test_ai_soldier(
        Camp::Lacklandists,
    ));
    engine.combat_event_ai_mut(owner).left_combat_neighbour =
        Some(crate::ai::AiEntityHandle::new(left.index()));
    engine.combat_event_ai_mut(owner).right_combat_neighbour =
        Some(crate::ai::AiEntityHandle::new(right.index()));
    engine.combat_event_ai_mut(left).right_combat_neighbour =
        Some(crate::ai::AiEntityHandle::new(owner.index()));
    engine.combat_event_ai_mut(right).left_combat_neighbour =
        Some(crate::ai::AiEntityHandle::new(owner.index()));
    engine.execute_ai_begin_swordfight(&crate::sim_rng::test_context(), &assets, owner);
    assert_eq!(engine.combat_event_ai(owner).left_combat_neighbour, None);
    assert_eq!(engine.combat_event_ai(owner).right_combat_neighbour, None);
    assert_eq!(engine.combat_event_ai(left).right_combat_neighbour, None);
    assert_eq!(engine.combat_event_ai(right).left_combat_neighbour, None);
}

#[test]
fn live_swordfight_entry_retains_selected_jump_line() {
    let (mut engine, assets, owner, target) = fixture(Substate::AttackingObserve);
    place(&mut engine, target, 120.0, 100.0, 0.0);
    for index in 0..3 {
        let mut line = crate::jump_line::JumpLine::new(
            MapPoint::new(90.0, 100.0),
            MapPoint::new(130.0, 100.0),
            0.0,
            0.0,
        );
        line.sector_index = crate::fast_find_grid::SectorIndex::new(0);
        line.associated_line_index = Some(if index == 1 { 2 } else { 1 });
        engine
            .world
            .fast_grid_mut()
            .level_mut()
            .jump_lines
            .push(line);
    }
    engine.combat_event_ai_mut(owner).my_line_jump = Some(1);
    engine.execute_ai_begin_swordfight(&crate::sim_rng::test_context(), &assets, owner);
    assert_eq!(engine.combat_event_ai(owner).my_line_jump, Some(1));
    assert!(engine.orders.sequence_manager.sequences_iter().flat_map(|s| s.elements.iter()).any(|element| {
        element.owner == Some(owner) && element.command == Command::EnterSwordfight
            && matches!(element.get_property(crate::sequence::Field::JumplineDestination), Some(crate::sequence::FieldValue::LineId(line)) if line.get() == 1)
    }));
}

fn fixture(substate: Substate) -> (EngineInner, LevelAssets, EntityId, EntityId) {
    let (mut engine, assets, owner, target) =
        crate::engine::ai::battle_decision_observation_tests::fixture(false);
    engine.combat_event_ai_mut(owner).base.current_substate = substate;
    for id in [owner, target] {
        let element = engine.get_entity_mut(id).unwrap().element_data_mut();
        let sector = element
            .sector()
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(0).unwrap());
        element.set_sector_topology(Some(sector), crate::fast_find_grid::SectorIndex::new(0));
        engine
            .get_entity_mut(id)
            .unwrap()
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
    }
    (engine, assets, owner, target)
}

fn place(engine: &mut EngineInner, id: EntityId, x: f32, y: f32, z: f32) {
    engine
        .get_entity_mut(id)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D::new(x, y, z));
}

fn event(engine: &mut EngineInner, assets: &LevelAssets, owner: EntityId, event: StimulusType) {
    let handled = engine.execute_ai_combat_expected_event(
        &crate::sim_rng::test_context(),
        assets,
        owner,
        event,
    );
    assert!(handled);
}

fn selected_door_position(engine: &mut EngineInner, owner: EntityId, point: MapPoint) {
    let sector = engine
        .expect_entity(owner, "door actor")
        .element_data()
        .sector()
        .unwrap();
    let gate = crate::gate::DoorIndex::new(engine.script_domains.interactables.doors.len() as u32)
        .unwrap();
    engine
        .script_domains
        .interactables
        .doors
        .push(crate::gate::Door {
            point_in: point,
            point_out: point,
            sector_in: crate::sector::SectorNumber::new(1),
            sector_out: crate::sector::SectorNumber::new(1),
            sector_in_index: sector.arena_index(),
            sector_out_index: sector.arena_index(),
            ..Default::default()
        });
    let mut element = crate::sequence::SequenceElement::new_movement(
        1,
        Command::PassDoor,
        Some(owner),
        OrderType::WalkingUpright,
    );
    let crate::sequence::SequenceElementData::Movement {
        gate_id, direction, ..
    } = &mut element.data
    else {
        unreachable!()
    };
    *gate_id = Some(gate);
    *direction = 1;
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        &mut Vec::new(),
        sequence,
        0,
    );
}

#[test]
fn approaching_new_enemy_uses_raw_distance_then_live_approach_position() {
    for (x, y, close) in [(110.0, 110.0, true), (83.73194, 32.8771, false)] {
        let (mut engine, mut assets, owner, target) =
            fixture(Substate::AttackingApproachingNewEnemy);
        std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].distance
            [crate::weapons::WeaponDistance::Default as usize] = 65;
        place(&mut engine, target, x, y, 0.0);
        selected_door_position(&mut engine, target, MapPoint::new(800.0, 800.0));
        event(&mut engine, &assets, owner, StimulusType::EventReachPoint);
        let ai = engine.combat_event_ai(owner);
        if close {
            assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
            assert_eq!(ai.base.when_does_timer_ring, 120);
        } else {
            assert_eq!(
                ai.base.current_substate,
                Substate::AttackingApproachingNewEnemy
            );
            assert_eq!(
                ai.base.last_goto_destination.map_point(),
                MapPoint::new(800.0, 800.0)
            );
            assert!(
                ai.base
                    .last_goto_flags
                    .contains(GotoFlags::NEAR | GotoFlags::RUN)
            );
        }
    }
}

#[test]
fn sleeping_enemy_distance_stretches_y_before_strike_or_approach() {
    for (dx, dy, close) in [(15.0, 34.0, false), (10.0, 10.0, true)] {
        let (mut engine, assets, owner, target) =
            fixture(Substate::AttackingApproachingSleepingEnemy);
        place(&mut engine, target, 100.0 + dx, 100.0 + dy, 0.0);
        engine
            .get_entity_mut(target)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .unconscious = true;
        event(&mut engine, &assets, owner, StimulusType::EventDone);
        let ai = engine.combat_event_ai(owner);
        assert_eq!(
            ai.base.current_substate,
            if close {
                Substate::AttackingKillingSleepingEnemy
            } else {
                Substate::AttackingApproachingSleepingEnemy
            }
        );
        let has_strike = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|s| s.elements.iter())
            .any(|e| e.owner == Some(owner) && e.command == Command::SwordstrikeDown);
        assert_eq!(has_strike, close);
        if !close {
            assert_eq!(
                ai.base.last_goto_destination.map_point(),
                MapPoint::new(115.0, 134.0)
            );
            assert!(ai.base.last_goto_flags.contains(GotoFlags::NEAR));
        }
    }
}

#[test]
fn reaction_timer_uses_raw_distance_and_installed_running_order() {
    for (target_point, running, expected) in [
        (
            (354.0, 731.0),
            false,
            crate::parameters_ai::AI_QUICK_ENEMY_REACTIONTIME as u32,
        ),
        ((380.0, 757.0), false, 1),
        (
            (380.0, 757.0),
            true,
            crate::parameters_ai::AI_RUNNING_ENEMY_REACTIONTIME as u32,
        ),
    ] {
        let (mut engine, assets, owner, target) = fixture(Substate::AttackingReactiontimeTurning);
        place(&mut engine, owner, 367.0, 757.0 + 480.0, 480.0);
        place(
            &mut engine,
            target,
            target_point.0,
            target_point.1 + 480.0,
            480.0,
        );
        selected_door_position(&mut engine, target, MapPoint::new(360.0, 750.0));
        engine
            .get_entity_mut(target)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .installed_order = Some(crate::element::InstalledActorOrder {
            order_id: std::num::NonZeroU32::new(1).unwrap(),
            order_type: if running {
                OrderType::RunningUpright
            } else {
                OrderType::WaitingUpright
            },
        });
        event(&mut engine, &assets, owner, StimulusType::EventDone);
        let ai = engine.combat_event_ai(owner);
        assert_eq!(ai.base.current_substate, Substate::AttackingReactiontime);
        assert_eq!(ai.base.when_does_timer_ring, 100 + expected);
    }
}

#[test]
fn ladder_and_roof_arrivals_keep_distinct_timers() {
    for (substate, result, delay) in [
        (
            Substate::AttackingRunningToLadder,
            Substate::AttackingWaitingAtLadder,
            1,
        ),
        (
            Substate::AttackingRunToAvengerOnRoof,
            Substate::AttackingWaitForAvengerOnRoof,
            100,
        ),
    ] {
        let (mut engine, assets, owner, target) = fixture(substate);
        let position = engine.live_ai_position(target);
        engine.combat_event_ai_mut(owner).base.seek_position = position;
        event(&mut engine, &assets, owner, StimulusType::EventReachPoint);
        let ai = engine.combat_event_ai(owner);
        assert_eq!(ai.base.current_substate, result);
        assert_eq!(ai.base.when_does_timer_ring, 100 + delay);
    }
}

#[test]
fn roof_timeout_seeks_from_live_owner_position() {
    let (mut engine, assets, owner, _) = fixture(Substate::AttackingWaitForAvengerOnRoof);
    let position = engine.live_ai_position(owner);
    let ai = engine.combat_event_ai_mut(owner);
    ai.base.primary_target = None;
    ai.base.seek_position = Position {
        x: 900.0,
        y: 900.0,
        ..position
    };
    engine.ai.global.seek_points = [110.0, 120.0, 130.0]
        .into_iter()
        .enumerate()
        .map(|(id, x)| crate::ai::SeekPoint {
            position: Position { x, ..position },
            frame_when_full_interest: 0,
            directions: vec![0],
            last_calculated_interest: 100,
            locked: false,
            id: id as u16,
        })
        .collect();
    event(&mut engine, &assets, owner, StimulusType::EventTimer);
    let ai = engine.combat_event_ai(owner);
    assert_eq!(ai.seek_center, position);
    assert!(ai.my_seek_points.iter().any(|id| *id < 3));
}

#[test]
fn door_fight_wait_timer_starts_observation() {
    let (mut engine, assets, owner, _) = fixture(Substate::AttackingDoorFightWaiting);
    event(&mut engine, &assets, owner, StimulusType::EventTimer);
    let ai = engine.combat_event_ai(owner);
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingOverviewLookLeft
    );
    assert!(ai.list_them.is_empty());
}

#[test]
fn archer_path_wait_returns_to_duty_only_on_timer() {
    for state in [
        Substate::AttackingArcherWaitOnArcheryPath,
        Substate::AttackingArcherWaitOnArcheryPathBending,
    ] {
        let (mut engine, assets, owner, _) = fixture(state);
        let handled = engine.execute_ai_combat_expected_event(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            StimulusType::EventDone,
        );
        assert!(!handled);
        assert_eq!(engine.combat_event_ai(owner).base.current_substate, state);
        event(&mut engine, &assets, owner, StimulusType::EventTimer);
        assert_eq!(
            engine.combat_event_ai(owner).base.current_state,
            AiState::Default
        );
    }
}

#[test]
fn bow_cover_arrival_faces_target_with_truncated_elevation() {
    let (mut engine, assets, owner, target) =
        fixture(Substate::AttackingBowRunningBehindShieldBearer);
    place(&mut engine, owner, 436.9325, 1227.554 + 45.0, 45.0);
    place(
        &mut engine,
        target,
        265.35767,
        1023.3345 + 151.12384,
        151.12384,
    );
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(3);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = crate::element::ActionState::Moving;
    let handled = engine.execute_ai_archery_expected_event(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        StimulusType::EventReachPoint,
    );
    assert!(handled);
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|s| s.elements.iter())
            .any(|e| e.owner == Some(owner)
                && matches!(
                    e.get_property(crate::sequence::Field::Direction),
                    Some(crate::sequence::FieldValue::Integer(14))
                ))
    );
}

#[test]
fn roof_timeout_refaces_visible_target_and_rearms_thirty_ticks() {
    let (mut engine, assets, owner, target) = fixture(Substate::AttackingWaitForAvengerOnRoof);
    place(&mut engine, target, 200.0, 100.0, 0.0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(crate::position_interface::vector_to_sector_0_to_15_iso(
            100.0, 0.0,
        ));
    let visible = engine.live_ai_detects_180(&assets, owner, target);
    assert!(visible, "roof fixture must admit its nearby target");
    event(&mut engine, &assets, owner, StimulusType::EventTimer);
    let ai = engine.combat_event_ai(owner);
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingWaitForAvengerOnRoof
    );
    assert_eq!(ai.base.when_does_timer_ring, 130);
}

#[test]
fn maximum_sword_range_wraps_squared_close_threshold_before_reapproach() {
    let (mut engine, mut assets, owner, target) = fixture(Substate::AttackingApproachingNewEnemy);
    std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].distance
        [crate::weapons::WeaponDistance::Default as usize] = u16::MAX;
    // The target's map position stays inside the arena, while raw Y/Z distance
    // exceeds the wrapped 32-bit threshold of 1,179,729.
    place(&mut engine, target, 900.0, 1100.0, 1000.0);
    let destination = engine.live_ai_position(target);
    event(&mut engine, &assets, owner, StimulusType::EventReachPoint);
    let ai = engine.combat_event_ai(owner);
    assert_eq!(
        ai.base.last_goto_destination, destination,
        "wrapping range arithmetic takes GoNear before its immediate-arrival branch"
    );
    assert!(
        ai.base
            .last_goto_flags
            .contains(GotoFlags::NEAR | GotoFlags::RUN)
    );
    assert_eq!(ai.base.stop_before_end_of_path_distance, u16::MAX);
    assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
    assert!(!ai.base.already_on_point);
}

#[test]
fn rider_retreat_direction_fifteen_wraps_to_zero_in_finite_arena() {
    let (mut engine, _, owner, _) = fixture(Substate::AttackingRiderChargingGettingDistance);
    place(&mut engine, owner, 500.0, 500.0, 0.0);
    let mut goals = Vec::new();
    for direction in [0, 15] {
        engine
            .get_entity_mut(owner)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(direction);
        goals.push(
            engine
                .combat_event_rider_retreat_goal(owner)
                .expect("finite arena admits retreat"),
        );
    }
    assert_eq!(
        goals[0], goals[1],
        "the fifteen-sector remainder maps heading 15 onto heading 0"
    );
    let vector = crate::coordinates::MapVec::from_sector_iso(0);
    let delta = MapPoint::new(goals[0].x - 500.0, goals[0].y - 500.0);
    assert_eq!(delta.x * vector.y - delta.y * vector.x, 0.0);
    assert!(delta.x * vector.x + delta.y * vector.y > 0.0);
}
