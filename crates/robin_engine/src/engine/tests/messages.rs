use super::*;

fn set_test_soldier_brawl_got_hit(engine: &mut EngineInner, soldier: EntityId) {
    use crate::ai::{AiState, Substate};

    let entity = engine
        .get_entity_mut(soldier)
        .expect("test soldier present");
    let npc = entity.npc_data_mut().expect("test soldier is an NPC");
    npc.ai_brain =
        crate::element::AiBrain::Enemy(Box::new(crate::ai_enemy::EnemyAi::new(soldier.index())));
    let ai = npc.ai_brain.enemy_mut().expect("enemy brain installed");
    ai.base.set_ai_state(AiState::Wondering);
    ai.base.current_substate = Substate::WonderingBrawlGotHit;
}

#[test]
fn completion_callback_recurses_before_returning() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::ai::{StimulusType, Substate};

    let mut engine = EngineInner::new();
    let soldier = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    set_test_soldier_brawl_got_hit(&mut engine, soldier);

    let assets = engine.test_runtime_assets();
    engine.execute_ai_callback(
        sim,
        &assets,
        soldier,
        &crate::ai::Stimulus::new(StimulusType::EventDone),
    );

    let ai = engine.get_entity(soldier).unwrap().ai_controller().unwrap();
    assert_eq!(
        ai.current_substate,
        Substate::WonderingWatchingForMoreMoney,
        "GotHit EventDone recursively fires EventDone in Recovering before the outer Think returns"
    );
    assert!(
        engine.orders.sequence_manager.sequences_iter().any(|seq| {
            seq.elements.iter().any(|elem| {
                matches!(
                    elem.command,
                    crate::element::Command::LookLeft | crate::element::Command::LookRight
                )
            })
        }),
        "the recursively selected look action must enter same-frame sequence arbitration"
    );
}

#[test]
fn change_way_tail_finishes_before_callers_next_callback() {
    use crate::ai::{AiState, MacroOpcode, StimulusType, Substate};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    let mut entity = make_test_civilian(crate::element::Posture::Upright);
    let Entity::Civilian(civilian_data) = &mut entity else {
        unreachable!("civilian fixture changed entity kind")
    };
    civilian_data.npc.ai_brain =
        crate::element::AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(0)));
    let civilian = engine.add_test_entity(entity);
    let paths = vec![RawHikingPath {
        waypoints: vec![RawWaypoint {
            x: 40,
            y: 20,
            sector: 1,
            level: 0,
            command: WaypointCommand::None,
        }],
    }];
    {
        let friendly = engine
            .get_entity_mut(civilian)
            .and_then(Entity::friendly_ai_mut)
            .expect("test civilian has Friendly AI");
        friendly.base.current_state = AiState::Default;
        friendly.base.current_substate = Substate::DefaultInMacro;
        friendly.base.macro_in_progress = true;
        friendly.base.macro_timer_is_running = true;
        friendly.base.macro_command = vec![MacroOpcode::ChangeWay as u8, 0, 0];
        friendly.base.number_of_remaining_macro_bytes = 3;
        friendly.fleeing_seen_enemy_counter = 5;
    }

    let mut assets = LevelAssets::new();
    assets.navigation.hiking_paths = std::sync::Arc::new(paths);
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.run_ai_macro(sim, &assets, civilian);
    {
        let friendly = engine
            .get_entity(civilian)
            .and_then(Entity::friendly_ai)
            .expect("test civilian retains Friendly AI");
        assert_eq!(friendly.fleeing_seen_enemy_counter, 0);
        assert!(!friendly.base.macro_in_progress);
        assert!(!friendly.base.macro_timer_is_running);
        assert!(
            !friendly
                .base
                .ai_log
                .iter()
                .any(|line| line.line_type == crate::ai::LogLineType::Event
                    && line.info == StimulusType::EventGaloppLoopEnd as u16)
        );
    }
    engine.execute_ai_callback(
        sim,
        &assets,
        civilian,
        &crate::ai::Stimulus::new(StimulusType::EventGaloppLoopEnd),
    );

    let friendly = engine
        .get_entity(civilian)
        .and_then(Entity::npc_data)
        .and_then(|npc| npc.ai_brain.friendly())
        .expect("test civilian retains Friendly AI");
    assert_eq!(friendly.fleeing_seen_enemy_counter, 0);
    assert!(!friendly.base.macro_in_progress);
    assert!(
        !friendly.base.macro_timer_is_running,
        "the delayed opcode tail must execute its explicit second macro interruption"
    );
    assert_eq!(
        friendly
            .base
            .ai_log
            .iter()
            .filter(|line| line.line_type == crate::ai::LogLineType::Event
                && line.info == StimulusType::EventGaloppLoopEnd as u16)
            .count(),
        1
    );
}

#[test]
fn change_way_suppressed_assignment_still_uses_friendly_virtual_tail() {
    use crate::ai::{AiState, MacroOpcode, Substate};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    let mut entity = make_test_civilian(crate::element::Posture::Upright);
    let Entity::Civilian(civilian_data) = &mut entity else {
        unreachable!("civilian fixture changed entity kind")
    };
    civilian_data.npc.ai_brain =
        crate::element::AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(0)));
    let civilian = engine.add_test_entity(entity);
    let paths = vec![RawHikingPath {
        waypoints: vec![RawWaypoint {
            x: 40,
            y: 20,
            sector: 1,
            level: 0,
            command: WaypointCommand::None,
        }],
    }];
    {
        let friendly = engine
            .get_entity_mut(civilian)
            .and_then(Entity::friendly_ai_mut)
            .expect("test civilian has Friendly AI");
        friendly.base.current_state = AiState::Wondering;
        friendly.base.current_substate = Substate::WonderingWatching;
        friendly.base.macro_in_progress = true;
        friendly.base.macro_timer_is_running = true;
        friendly.base.macro_command = vec![MacroOpcode::ChangeWay as u8, 0, 0];
        friendly.base.number_of_remaining_macro_bytes = 3;
        friendly.fleeing_seen_enemy_counter = 7;
    }

    let mut assets = LevelAssets::new();
    assets.navigation.hiking_paths = std::sync::Arc::new(paths);
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.run_ai_macro(sim, &assets, civilian);

    let friendly = engine
        .get_entity(civilian)
        .and_then(Entity::npc_data)
        .and_then(|npc| npc.ai_brain.friendly())
        .expect("test civilian retains Friendly AI");
    assert_eq!(friendly.fleeing_seen_enemy_counter, 0);
    assert_eq!(friendly.base.current_state, AiState::Default);
    assert!(!friendly.base.macro_timer_is_running);
}

#[test]
fn change_way_enemy_assignment_consumes_ale_before_explicit_patrol_tail() {
    use crate::ai::{AiState, MacroOpcode, Position, Substate};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    let soldier = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    let ale_destination = crate::coordinates::MapPoint::new(40.0, 20.0);
    let mut ale_element = {
        let mut initial_element = crate::element::ElementData::default();
        initial_element.kind = crate::element::ElementKind::ObjectOther;
        initial_element.active = true;
        initial_element
    };
    ale_element.set_position_map(ale_destination);
    ale_element.set_sector(crate::ai::SectorHandle::new(1));
    let ale = engine.add_test_entity(Entity::Bonus(crate::element::ElementBonus {
        element: ale_element,
        object: crate::element::ObjectData {
            object_type: crate::element::ObjectType::Ale,
            ..crate::element::ObjectData::default()
        },
    }));
    {
        let npc = engine
            .get_entity_mut(soldier)
            .and_then(Entity::npc_data_mut)
            .expect("test soldier is NPC");
        npc.ai_brain = crate::element::AiBrain::Enemy(Box::new(crate::ai_enemy::EnemyAi::new(
            soldier.index(),
        )));
    }
    let paths = vec![RawHikingPath {
        waypoints: vec![
            RawWaypoint {
                x: 100,
                y: 20,
                sector: 1,
                level: 0,
                command: WaypointCommand::None,
            },
            RawWaypoint {
                x: 180,
                y: 20,
                sector: 1,
                level: 0,
                command: WaypointCommand::None,
            },
        ],
    }];
    {
        let soldier_element = engine
            .get_entity_mut(soldier)
            .expect("test soldier present")
            .element_data_mut();
        soldier_element.set_position_map(crate::coordinates::MapPoint::new(0.0, 20.0));
        soldier_element.set_sector(crate::ai::SectorHandle::new(1));
    }
    {
        let enemy = engine
            .get_entity_mut(soldier)
            .and_then(Entity::enemy_ai_mut)
            .expect("test soldier has Enemy AI");
        enemy.base.current_state = AiState::Default;
        enemy.base.current_substate = Substate::DefaultInMacro;
        enemy.base.macro_in_progress = true;
        enemy.base.macro_command = vec![MacroOpcode::ChangeWay as u8, 0, 0];
        enemy.base.number_of_remaining_macro_bytes = 3;
        enemy.other_seen_ale.push(ale.index());
    }

    let mut assets = LevelAssets::new();
    assets.navigation.hiking_paths = std::sync::Arc::new(paths);
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.run_ai_macro(sim, &assets, soldier);

    let ai = engine
        .get_entity(soldier)
        .and_then(Entity::ai_controller)
        .expect("test soldier retains AI");
    assert_eq!(ai.current_substate, Substate::DefaultGotoRoute);
    assert!(!ai.macro_timer_is_running);
    assert!(
        !ai.timer_is_running,
        "A's ale timer must be cleared when B's enemy state change returns to the patrol route"
    );
    let enemy = engine
        .get_entity(soldier)
        .and_then(Entity::enemy_ai)
        .expect("test soldier retains Enemy AI");
    assert!(enemy.other_seen_ale.is_empty());
    assert_eq!(
        enemy.base.last_goto_destination,
        Position {
            x: 100.0,
            y: 20.0,
            sector: crate::ai::SectorHandle::new(1),
            level: 0,
        },
        "B must fall through to the patrol waypoint after A consumes the ale"
    );
    let mut movements: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .filter_map(|sequence| {
            let element = sequence
                .elements
                .iter()
                .find(|element| element.owner == Some(soldier) && element.data.is_movement())?;
            let crate::sequence::SequenceElementData::Movement { destination, .. } = &element.data
            else {
                unreachable!("movement predicate admitted non-movement data")
            };
            Some((sequence.id, *destination))
        })
        .collect();
    movements.sort_by_key(|(sequence_id, _)| sequence_id.0);
    assert_eq!(movements.len(), 2);
    assert_eq!(
        [movements[0].0.0, movements[1].0.0],
        [1, 2],
        "A and B must register exactly once and in causal order on the fresh manager"
    );
    assert_eq!(
        engine.world.entities.current_element_for_actor(soldier),
        None,
        "owner-work drain registers both moves before the later manager selection phase"
    );
    assert_eq!(movements[0].1.x, 100.0);
    assert_eq!(movements[0].1.y, 20.0);
    assert_eq!(movements[1].1.x, 100.0);
    assert_eq!(movements[1].1.y, 20.0);
}

#[test]
fn condolation_reenters_think_before_dispatch_returns() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::ai::Substate;
    use crate::element::Command;
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let soldier = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    set_test_soldier_brawl_got_hit(&mut engine, soldier);

    let assets = engine.test_runtime_assets();
    let seq_id = engine.launch_element(
        &crate::sim_rng::test_context(),
        &assets,
        SequenceElement::new(1, Command::LookLeft, Some(soldier)),
    );
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        seq_id,
        0,
    );
    engine.element_terminated(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        seq_id,
        0,
    );

    let ai = engine.get_entity(soldier).unwrap().ai_controller().unwrap();
    assert_eq!(
        ai.current_substate,
        Substate::WonderingWatchingForMoreMoney,
        "state change, removal notification, and completion-event decision tick must finish before dispatch returns"
    );
    assert!(
        engine.orders.sequence_manager.sequences_iter().any(|seq| {
            seq.elements.iter().any(|elem| {
                matches!(
                    elem.command,
                    crate::element::Command::LookLeft | crate::element::Command::LookRight
                )
            })
        }),
        "condolation re-entry must launch its follow-up before dispatch returns"
    );
}

#[test]
fn halt_condolation_clears_only_the_selected_movement_goal() {
    use crate::coordinates::MapPoint;
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{CascadeFlags, SequenceElement};

    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_soldier(Posture::Upright));

    let movement =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    let movement_seq = engine.launch_element(&crate::sim_rng::test_context(), &assets, movement);
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        movement_seq,
        0,
    );
    {
        let entity = engine.get_entity_mut(owner).unwrap();

        entity
            .position_iface_mut()
            .set_map_goal(MapPoint::new(70.0, 80.0));
    }

    // An unrelated card for the same owner can be delivered while the
    // movement remains selected (for example, postponed parallel work).
    // Base actor condolence dispatch compares selected-element identity
    // before detaching the current movement.
    let unrelated_seq = engine.launch_element(
        &crate::sim_rng::test_context(),
        &assets,
        SequenceElement::new(1, Command::LookLeft, Some(owner)),
    );
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        unrelated_seq,
        0,
    );
    engine.select_sequence_element(owner, Some((movement_seq, 0)));
    engine.orders.sequence_manager.set_halt_pending(true);
    engine.element_interrupted(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        unrelated_seq,
        0,
        CascadeFlags::NEXT_LEVEL,
    );
    engine.orders.sequence_manager.set_halt_pending(false);

    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(
        engine.world.entities.current_element_for_actor(owner),
        Some((movement_seq, 0))
    );
    assert_eq!(
        entity.position_iface().map_goal(),
        MapPoint::new(70.0, 80.0),
        "an unrelated halt card must not detach the selected movement"
    );

    engine.orders.sequence_manager.set_halt_pending(true);
    engine.element_interrupted(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        movement_seq,
        0,
        CascadeFlags::NEXT_LEVEL,
    );
    engine.orders.sequence_manager.set_halt_pending(false);

    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(
        entity.position_iface().map_goal(),
        MapPoint::ZERO,
        "actor-base halt cleanup clears the selected movement goal before the NPC halt guard"
    );
}

#[test]
fn selected_nonmovement_condolation_clears_the_sprite_goal() {
    use crate::coordinates::MapPoint;
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_soldier(Posture::OnWall));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .position_iface_mut()
        .set_map_goal(MapPoint::new(70.0, 80.0));

    let assert_position = SequenceElement::new_movement(
        1,
        Command::AssertPosition,
        Some(owner),
        OrderType::WalkingUpright,
    );
    let sequence = engine.launch_element(&crate::sim_rng::test_context(), &assets, assert_position);
    engine.select_sequence_element(owner, Some((sequence, 0)));
    engine.element_terminated(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        sequence,
        0,
    );

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        MapPoint::ZERO,
        "a selected AssertPosition card clears the old movement goal before its successor executes"
    );
}

#[test]
fn interrupted_movement_clears_goal_before_next_wait_is_selected() {
    use crate::coordinates::MapPoint;
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{CascadeFlags, SequenceElement, SequencePriority};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_soldier(Posture::Upright));
    let goal = MapPoint::new(536.9613, 447.9872);

    let movement =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::WalkingUpright);
    let movement_sequence =
        engine.launch_element(&crate::sim_rng::test_context(), &assets, movement);
    engine.select_sequence_element(owner, Some((movement_sequence, 0)));
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        movement_sequence,
        0,
    );
    {
        let entity = engine.get_entity_mut(owner).unwrap();

        entity.position_iface_mut().set_map_goal(goal);
    }

    // Interruption closes the selected movement before its caller selects Wait.
    engine.element_interrupted(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        movement_sequence,
        0,
        CascadeFlags::NEXT_LEVEL,
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        MapPoint::ZERO
    );
    let mut wait = SequenceElement::new(1, Command::Wait, Some(owner));
    wait.priority = SequencePriority::Wait;
    let wait_sequence = engine.launch_element(&crate::sim_rng::test_context(), &assets, wait);
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        wait_sequence,
        0,
    );

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        MapPoint::ZERO,
        "selecting Wait must preserve the completed movement cleanup"
    );
    assert_eq!(
        engine.world.entities.current_element_for_actor(owner),
        Some((wait_sequence, 0)),
        "the caller's subsequent Wait remains selected"
    );
}

#[test]
fn attentive_postpone_current_preserves_rewritten_movement_goal() {
    use crate::coordinates::MapPoint;
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_soldier(Posture::Upright));
    let goal = MapPoint::new(1183.0403, 743.6907);

    let mut movement =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::WalkingUpright);
    movement.priority = SequencePriority::Normal;
    movement.orders.push_back(Order::new(
        OrderType::WalkingUpright,
        goal.x,
        goal.y,
        engine.orders.allocate_order_id(),
    ));
    let movement_sequence =
        engine.launch_element(&crate::sim_rng::test_context(), &assets, movement);
    engine.select_sequence_element(owner, Some((movement_sequence, 0)));
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        movement_sequence,
        0,
    );
    {
        let entity = engine.get_entity_mut(owner).unwrap();

        entity.position_iface_mut().set_map_goal(goal);
    }

    // Preference-priority actor stopping rewrites Walking to the stopping transition but
    // deliberately keeps the selected movement alive. The stronger attentive
    // command then POSTPONE_CURRENTs it without a condolence card.
    engine.stop_actor_orders(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        owner,
        SequencePriority::Preference,
    );
    engine.set_soldier_attentive_mode(&crate::sim_rng::test_context(), &assets, owner, true, false);
    // The attentive element is only registered here; drive the manager
    // update so its deferred instruction performs the POSTPONE_CURRENT.
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut crate::engine::HostDisplayState::default(),
        &LevelAssets::new(),
    );

    let movement = engine
        .orders
        .sequence_manager
        .get_element(movement_sequence, 0)
        .expect("postponed movement remains registered");
    assert_eq!(movement.state, SequenceState::Postponed);
    assert_eq!(movement.command, Command::Move);
    assert!(movement.orders.is_empty());
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        goal,
        "POSTPONE_CURRENT has no selected-element condolence and retains the movement goal"
    );
}

#[test]
fn completed_immediate_sibling_does_not_clear_selected_movement_goal() {
    use crate::coordinates::MapPoint;
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_soldier(Posture::Upright));
    let goal = MapPoint::new(70.0, 80.0);

    let movement =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    let movement_sequence =
        engine.launch_element(&crate::sim_rng::test_context(), &assets, movement);
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        movement_sequence,
        0,
    );
    {
        let entity = engine.get_entity_mut(owner).unwrap();

        entity.position_iface_mut().set_map_goal(goal);
    }

    let sibling = SequenceElement::new(1, Command::SpeakHeroReachDestination, Some(owner));
    let sibling_sequence = engine.launch_element(&crate::sim_rng::test_context(), &assets, sibling);
    // The player speech override terminates before delegating to actor instruction handling,
    // so it never replaces the selected movement pointer.
    engine.element_terminated(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        sibling_sequence,
        0,
    );

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        goal,
        "a finished immediate sibling must not clear the movement that is selected again when its callback returns"
    );
}

#[test]
fn pc_arrival_speech_finishes_before_non_interruptable_postponement() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{Sequence, SequenceElement, SequencePriority, SequenceState};

    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_pc(Posture::SimulatingBeggar));

    let mut leave_beggar = SequenceElement::new(1, Command::LeaveBeggar, Some(owner));
    leave_beggar.priority = SequencePriority::NonInterruptable;
    let blocker = engine.launch_element(&crate::sim_rng::test_context(), &assets, leave_beggar);
    engine.select_sequence_element(owner, Some((blocker, 0)));
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        blocker,
        0,
    );

    let mut sequence = Sequence::new();
    sequence.append_element(SequenceElement::new_movement(
        1,
        Command::Move,
        Some(owner),
        OrderType::WalkingUpright,
    ));
    sequence.append_element(SequenceElement::new(
        1,
        Command::SpeakHeroReachDestination,
        Some(owner),
    ));
    sequence.append_element(SequenceElement::new(2, Command::EnterBeggar, Some(owner)));
    let movement = engine.launch_sequence(&crate::sim_rng::test_context(), &assets, sequence);

    assert!(engine.instruct_owner(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        &mut Vec::new(),
        owner,
        movement,
        0
    ));
    assert!(engine.instruct_owner(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        &mut Vec::new(),
        owner,
        movement,
        1
    ));

    let sequence = engine
        .orders
        .sequence_manager
        .get_sequence(movement)
        .expect("movement sequence survives while its move is postponed");
    assert_eq!(sequence.elements[0].state, SequenceState::Postponed);
    assert_eq!(sequence.elements[1].state, SequenceState::Terminated);
    assert_eq!(
        sequence.elements[2].state,
        SequenceState::Todo,
        "terminating the same-level PC speech must not cascade Impossible into posture recovery"
    );
}

#[test]
fn interrupted_movement_preserves_goal_when_incoming_action_is_selected() {
    use crate::coordinates::MapPoint;
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{CascadeFlags, SequenceElement};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_soldier(Posture::Upright));
    let goal = MapPoint::new(1004.836, 1774.2802);

    let movement =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    let movement_seq = engine.launch_element(&crate::sim_rng::test_context(), &assets, movement);
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        movement_seq,
        0,
    );
    {
        let entity = engine.get_entity_mut(owner).unwrap();

        entity.position_iface_mut().set_map_goal(goal);
    }

    let incoming_seq = engine.launch_element(
        &crate::sim_rng::test_context(),
        &assets,
        SequenceElement::new(1, Command::EnterAttentiveMode, Some(owner)),
    );
    engine.select_sequence_element(owner, Some((incoming_seq, 0)));
    engine.element_interrupted(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        movement_seq,
        0,
        CascadeFlags::NEXT_LEVEL,
    );

    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(
        entity.position_iface().map_goal(),
        goal,
        "Original clears the sprite goal only when the outgoing movement is still selected"
    );
}

#[test]
fn halt_condolation_does_not_instruct_a_registered_replacement_move() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{CascadeFlags, SequenceElement};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_soldier(Posture::Upright));
    let outgoing =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    let outgoing_seq = engine.launch_element(&crate::sim_rng::test_context(), &assets, outgoing);
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        outgoing_seq,
        0,
    );

    let replacement =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    let replacement_seq =
        engine.launch_element(&crate::sim_rng::test_context(), &assets, replacement);

    engine.orders.sequence_manager.set_halt_pending(true);
    engine.element_interrupted(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        outgoing_seq,
        0,
        CascadeFlags::NEXT_LEVEL,
    );
    engine.orders.sequence_manager.set_halt_pending(false);

    assert!(
        engine
            .orders
            .sequence_manager
            .deferred_elements_to_go()
            .contains(&(replacement_seq, 0)),
        "a Halt card suppresses Think and leaves the replacement pending manager instruction"
    );
}

#[test]
fn condolation_followup_arbitrates_before_parent_sequence_successor() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::ai::{AiState, Substate};
    use crate::element::Command;
    use crate::sequence::{Sequence, SequenceAction, SequenceElement};

    let mut engine = EngineInner::new();
    let soldier = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    set_test_soldier_brawl_got_hit(&mut engine, soldier);

    let mut parent = Sequence::new();
    parent.append_element(SequenceElement::new(1, Command::LookLeft, Some(soldier)));
    // Final-action detection explicitly skips Wait/AssertPosition successors,
    // so the LookLeft condolence still fires before Ready queues this.
    parent.append_element(SequenceElement::new(2, Command::Wait, Some(soldier)));
    let assets = engine.test_runtime_assets();
    let parent_id = engine.launch_sequence(&crate::sim_rng::test_context(), &assets, parent);

    let initial = std::iter::from_fn(|| engine.orders.sequence_manager.pop_next_hourglass_action())
        .collect::<Vec<_>>();
    assert_eq!(initial.len(), 1);
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        parent_id,
        0,
    );
    engine.element_terminated(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        parent_id,
        0,
    );

    let commands: Vec<_> =
        std::iter::from_fn(|| engine.orders.sequence_manager.pop_next_hourglass_action())
            .collect::<Vec<_>>()
            .into_iter()
            .map(|action| {
                let (seq_id, elem_idx) = match action {
                    SequenceAction::InstructOwner {
                        sequence_id,
                        element_index,
                        ..
                    }
                    | SequenceAction::EngineCommand {
                        sequence_id,
                        element_index,
                    }
                    | SequenceAction::ExecuteImmediateOwner {
                        sequence_id,
                        element_index,
                        ..
                    }
                    | SequenceAction::ExecuteImmediateEngine {
                        sequence_id,
                        element_index,
                    } => (sequence_id, element_index),
                };
                engine
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .expect("queued action still has an element")
                    .command
            })
            .collect();

    assert_eq!(
        commands,
        vec![
            Command::EnterAttentiveMode,
            Command::LookLeft,
            Command::Wait,
        ],
        "completion notification's recursive AI processing must launch/arbitrate its action before queuing the parent's next level"
    );

    let ai = engine.get_entity(soldier).unwrap().ai_controller().unwrap();
    assert_eq!(ai.current_state, AiState::Wondering);
    assert_eq!(ai.current_substate, Substate::WonderingWatchingForMoreMoney);
}

#[test]
fn condolation_cascade_crosses_owners_before_outer_dispatch_returns() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::element::Command;
    use crate::sequence::{CascadeFlags, Sequence, SequenceElement, SequenceState};

    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let first = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    let second = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    let third = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    for owner in [second, third] {
        engine
            .get_entity_mut(owner)
            .unwrap()
            .npc_data_mut()
            .unwrap()
            .wasp_victim = true;
    }

    let mut seq = Sequence::new();
    seq.append_element(SequenceElement::new(1, Command::LookLeft, Some(first)));
    seq.append_element(SequenceElement::new(
        2,
        Command::ReceiveWaspSting,
        Some(second),
    ));
    seq.append_element(SequenceElement::new(
        3,
        Command::ReceiveWaspSting,
        Some(third),
    ));
    let seq_id = engine.launch_sequence(&crate::sim_rng::test_context(), &assets, seq);

    engine.element_interrupted(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        seq_id,
        0,
        CascadeFlags::NEXT_LEVEL,
    );

    for (idx, owner) in [(1, second), (2, third)] {
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, idx)
                .unwrap()
                .state,
            SequenceState::Interrupted
        );
        assert!(
            !engine
                .get_entity(owner)
                .unwrap()
                .npc_data()
                .unwrap()
                .wasp_victim,
            "cross-owner notification {idx} must run inside the originating state-change cascade"
        );
    }
}

#[test]
fn condolation_ready_executes_immediate_timer_successor_inline() {
    use crate::element::Command;
    use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};

    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));

    // The mission regression had three actor elements at the current command
    // level and an ownerless Timer at index 3.  The last actor condolence
    // resumes readiness, which must execute that timer before the state change returns.
    let mut sequence = Sequence::new();
    for command in [Command::LookLeft, Command::LookRight, Command::LookLeft] {
        sequence.append_element(SequenceElement::new(1, command, Some(owner)));
    }
    let mut timer = SequenceElement::new_generic(2, Command::Timer, None);
    timer.set_property(Field::Timer, FieldValue::Integer(12));
    sequence.append_element(timer);
    let sequence_id = engine.launch_sequence(&crate::sim_rng::test_context(), &assets, sequence);
    let initial = std::iter::from_fn(|| engine.orders.sequence_manager.pop_next_hourglass_action())
        .collect::<Vec<_>>();
    assert_eq!(initial.len(), 3);

    // Suppress the AI EventDone callbacks just as Halt does; the regression is
    // the continuation of state change after removal notification returns.
    engine.orders.sequence_manager.set_halt_pending(true);
    for element_index in 0..3 {
        engine.element_in_progress(
            &crate::sim_rng::test_context(),
            &assets,
            &mut Vec::new(),
            sequence_id,
            element_index,
        );
        engine.element_terminated(
            &crate::sim_rng::test_context(),
            &assets,
            &mut Vec::new(),
            sequence_id,
            element_index,
        );
    }
    engine.orders.sequence_manager.set_halt_pending(false);

    assert_eq!(engine.orders.timer_elements.len(), 1);
    assert_eq!(engine.orders.timer_elements[0].remaining, 12);
}
