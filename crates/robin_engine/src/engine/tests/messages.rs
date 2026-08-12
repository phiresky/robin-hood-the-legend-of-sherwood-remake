use super::*;

fn set_test_soldier_brawl_got_hit(engine: &mut EngineInner, soldier: EntityId) {
    use crate::ai::{AiState, Substate};

    let entity = engine
        .get_entity_mut(soldier)
        .expect("test soldier present");
    let npc = entity.npc_data_mut().expect("test soldier is an NPC");
    npc.ai_brain =
        crate::element::AiBrain::Enemy(Box::new(crate::ai_enemy::EnemyAi::new(soldier.index())));
    npc.ai_brain
        .enemy_mut()
        .expect("enemy brain installed")
        .set_state(AiState::Wondering, Substate::WonderingBrawlGotHit);
}

#[test]
fn self_stimulus_chain_reenters_until_stable_in_originating_frame() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::ai::{StimulusType, Substate};

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    set_test_soldier_brawl_got_hit(&mut engine, soldier);
    engine
        .get_entity_mut(soldier)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .fire_self_stimulus(StimulusType::EventDone);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.drain_pending_self_stimuli(sim, &assets);

    let ai = engine.get_entity(soldier).unwrap().ai_controller().unwrap();
    assert_eq!(
        ai.current_substate,
        Substate::WonderingWatchingForMoreMoney,
        "GotHit EventDone recursively fires EventDone in Recovering before the outer Think returns"
    );
    assert!(
        ai.outbox.reentrant.self_stimuli.is_empty(),
        "a recursive self-stimulus must not leak into the next frame"
    );
    assert!(ai.outbox.actor.look_sidewards.is_none());
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
fn post_reentrant_macro_cleanup_preserves_nested_wait_deadline() {
    use crate::ai::{AiState, MacroOpcode, StimulusType, Substate};

    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    engine.control.frame_counter = 458;
    let mut entity = make_test_civilian(crate::element::Posture::Upright);
    let Entity::Civilian(civilian_data) = &mut entity else {
        unreachable!("civilian fixture changed entity kind")
    };
    civilian_data.npc.ai_brain =
        crate::element::AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(0)));
    let civilian = engine.add_entity(entity);
    {
        let ai = engine
            .get_entity_mut(civilian)
            .and_then(Entity::ai_controller_mut)
            .expect("test civilian has AI");
        ai.current_state = AiState::Default;
        ai.current_substate = Substate::DefaultInMacroWaitingForDone;
        ai.macro_command = vec![MacroOpcode::Wait as u8, 50, 0];
        ai.macro_command_offset = 0;
        ai.number_of_remaining_macro_bytes = 3;
        ai.macro_in_progress = true;
        ai.macro_timer_is_running = false;
        ai.outbox
            .reentrant
            .self_stimuli
            .push(StimulusType::EventDone);
        ai.outbox.reentrant.finish_macro_after_self_stimuli = true;
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.drain_self_stimuli_for_npc(sim, civilian, &assets);

    let ai = engine
        .get_entity(civilian)
        .and_then(Entity::ai_controller)
        .expect("test civilian retains AI");
    assert_eq!(ai.when_does_macro_timer_ring, 508);
    assert!(!ai.macro_in_progress);
    assert!(!ai.macro_timer_is_running);
    assert!(!ai.outbox.reentrant.finish_macro_after_self_stimuli);
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());
}

#[test]
fn change_way_tail_runs_between_assignment_callback_and_existing_sibling() {
    use crate::ai::{AiContext, AiState, MacroOpcode, Position, StimulusType, Substate};
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
    let civilian = engine.add_entity(entity);
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
        friendly
            .base
            .outbox
            .reentrant
            .self_stimuli
            .push(StimulusType::EventPanic);
        friendly.base.execute_next_macro_command(
            sim,
            &AiContext {
                position: Position::default(),
                hiking_paths: std::sync::Arc::new(paths.clone()),
                ..AiContext::default()
            },
        );
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.drain_ai_owner_work_for(sim, &assets, civilian);
    assert_eq!(
        engine
            .get_entity(civilian)
            .and_then(Entity::ai_controller)
            .expect("test civilian retains AI")
            .outbox
            .reentrant
            .self_stimuli,
        [StimulusType::EventPanic],
        "unrelated sibling C must remain queued until explicit virtual tail B has returned"
    );
    engine.drain_self_stimuli_for_npc(sim, civilian, &assets);

    let friendly = engine
        .get_entity(civilian)
        .and_then(Entity::npc_data)
        .and_then(|npc| npc.ai_brain.friendly())
        .expect("test civilian retains Friendly AI");
    assert_eq!(friendly.fleeing_seen_enemy_counter, 0);
    assert!(!friendly.base.macro_in_progress);
    assert!(
        !friendly.base.macro_timer_is_running,
        "the delayed opcode tail must execute its explicit second BreakMacro"
    );
    assert!(friendly.base.outbox.reentrant.self_stimuli.is_empty());
}

#[test]
fn change_way_suppressed_assignment_still_uses_friendly_virtual_tail() {
    use crate::ai::{AiContext, AiState, MacroOpcode, Position, Substate};
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
    let civilian = engine.add_entity(entity);
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
        friendly.base.execute_next_macro_command(
            sim,
            &AiContext {
                position: Position::default(),
                hiking_paths: std::sync::Arc::new(paths.clone()),
                ..AiContext::default()
            },
        );
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.drain_ai_owner_work_for(sim, &assets, civilian);

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
    use crate::ai::{AiContext, AiState, MacroOpcode, Position, Substate};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    let ale_destination = crate::coordinates::MapPoint::new(40.0, 20.0);
    let mut ale_element = crate::element::ElementData {
        kind: crate::element::ElementKind::ObjectOther,
        active: true,
        ..crate::element::ElementData::default()
    };
    ale_element.set_position_map(ale_destination);
    ale_element.set_sector(crate::ai::SectorHandle::new(1));
    let ale = engine.add_entity(Entity::Bonus(crate::element::ElementBonus {
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
        enemy.base.execute_next_macro_command(
            sim,
            &AiContext {
                position: Position {
                    x: 0.0,
                    y: 20.0,
                    sector: crate::ai::SectorHandle::new(1),
                    level: 0,
                },
                self_is_soldier: true,
                hiking_paths: std::sync::Arc::new(paths.clone()),
                ..AiContext::default()
            },
        );
    }

    let mut assets = LevelAssets::new();
    assets.hiking_paths = std::sync::Arc::new(paths);
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.drain_ai_owner_work_for(sim, &assets, soldier);

    let ai = engine
        .get_entity(soldier)
        .and_then(Entity::ai_controller)
        .expect("test soldier retains AI");
    assert!(ai.outbox.actor.orders.is_empty());
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());
    assert!(ai.outbox.reentrant.owner_work.is_empty());
    assert!(!ai.has_pending_synchronous_cross_npc_actions());
    assert_eq!(ai.current_substate, Substate::DefaultGotoRoute);
    assert!(!ai.macro_timer_is_running);
    assert!(
        !ai.timer_is_running,
        "A's ale timer must be cleared when B's virtual Enemy SetState returns to the patrol route"
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
    assert!(
        movements[0].0.0 >= 3,
        "A's ale GoTo must consume sequence IDs before B replaces it with the live patrol route"
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
    let soldier = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    set_test_soldier_brawl_got_hit(&mut engine, soldier);

    let seq_id = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::LookLeft, Some(soldier)));
    engine
        .orders
        .sequence_manager
        .element_in_progress(seq_id, 0);
    engine.orders.sequence_manager.element_terminated(seq_id, 0);
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.dispatch_condolations(sim, &assets);

    let ai = engine.get_entity(soldier).unwrap().ai_controller().unwrap();
    assert_eq!(
        ai.current_substate,
        Substate::WonderingWatchingForMoreMoney,
        "SetState -> SendCondolationCard -> Think(EventDone) must finish before dispatch returns"
    );
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());
    assert!(ai.outbox.actor.look_sidewards.is_none());
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
    use crate::movement::ActiveMovement;
    use crate::order::OrderType;
    use crate::sequence::{CascadeFlags, SequenceElement};

    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let movement =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    let movement_seq = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_seq, 0);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.actor_data_mut().unwrap().active_movement = ActiveMovement::new(movement_seq, 0);
        entity
            .position_iface_mut()
            .set_map_goal(MapPoint::new(70.0, 80.0));
    }

    // An unrelated card for the same owner can be delivered while the
    // movement remains selected (for example, postponed parallel work).
    // Actor-base SendCondolationCard compares mpSequenceElement identity
    // before detaching the current movement.
    let unrelated_seq = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::LookLeft, Some(owner)));
    engine
        .orders
        .sequence_manager
        .element_in_progress(unrelated_seq, 0);
    engine.orders.sequence_manager.set_halt_pending(true);
    engine
        .orders
        .sequence_manager
        .element_interrupted(unrelated_seq, 0, CascadeFlags::NEXT_LEVEL);
    engine.orders.sequence_manager.set_halt_pending(false);
    engine.dispatch_condolations(sim, &LevelAssets::new());

    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(
        entity.actor_data().unwrap().active_movement,
        ActiveMovement::new(movement_seq, 0)
    );
    assert_eq!(
        entity.position_iface().map_goal(),
        MapPoint::new(70.0, 80.0),
        "an unrelated halt card must not detach the selected movement"
    );

    engine.orders.sequence_manager.set_halt_pending(true);
    engine
        .orders
        .sequence_manager
        .element_interrupted(movement_seq, 0, CascadeFlags::NEXT_LEVEL);
    engine.orders.sequence_manager.set_halt_pending(false);
    engine.dispatch_condolations(sim, &LevelAssets::new());

    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(
        entity.actor_data().unwrap().active_movement,
        ActiveMovement::none(),
        "the selected movement's halt card detaches active movement"
    );
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
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::OnWall));
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
    let sequence = engine
        .orders
        .sequence_manager
        .launch_element(assert_position);
    engine
        .orders
        .sequence_manager
        .begin_instruct_callback(owner, sequence, 0);
    engine
        .orders
        .sequence_manager
        .element_terminated(sequence, 0);
    engine
        .orders
        .sequence_manager
        .end_instruct_callback(owner, sequence, 0);
    engine.dispatch_condolations(&sim, &LevelAssets::new());

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
fn delayed_selected_movement_card_clears_goal_after_wait_is_selected() {
    use crate::coordinates::MapPoint;
    use crate::element::{Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::OrderType;
    use crate::sequence::{CascadeFlags, SequenceElement, SequencePriority};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    let goal = MapPoint::new(536.9613, 447.9872);

    let movement =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::WalkingUpright);
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.actor_data_mut().unwrap().active_movement =
            ActiveMovement::new(movement_sequence, 0);
        entity.position_iface_mut().set_map_goal(goal);
    }

    // SetState snapshots that this was the selected element, but Rust queues
    // the condolence instead of invoking it recursively. A later actor slot
    // can install Wait before the queue is drained.
    engine.orders.sequence_manager.element_interrupted(
        movement_sequence,
        0,
        CascadeFlags::NEXT_LEVEL,
    );
    let mut wait = SequenceElement::new(1, Command::Wait, Some(owner));
    wait.priority = SequencePriority::Wait;
    let wait_sequence = engine.orders.sequence_manager.launch_element(wait);
    engine
        .orders
        .sequence_manager
        .element_in_progress(wait_sequence, 0);

    engine.dispatch_condolations(&sim, &LevelAssets::new());

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        MapPoint::ZERO,
        "terminal-time selected identity must survive delayed card dispatch"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner),
        Some((wait_sequence, 0)),
        "delayed cleanup must not detach the newly selected Wait"
    );
}

#[test]
fn attentive_postpone_current_preserves_rewritten_movement_goal() {
    use crate::coordinates::MapPoint;
    use crate::element::{Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
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
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.actor_data_mut().unwrap().active_movement =
            ActiveMovement::new(movement_sequence, 0);
        entity.position_iface_mut().set_map_goal(goal);
    }

    // Actor::Stop(Preference) rewrites Walking to the stopping transition but
    // deliberately keeps the selected movement alive. The stronger attentive
    // command then POSTPONE_CURRENTs it without a condolence card.
    engine.stop_owner(owner, SequencePriority::Preference);
    engine.set_soldier_attentive_mode(owner, true, false);
    // The attentive element is only registered here; drive the manager
    // hourglass so its deferred Instruct performs the POSTPONE_CURRENT.
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
    use crate::movement::ActiveMovement;
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    let goal = MapPoint::new(70.0, 80.0);

    let movement =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.actor_data_mut().unwrap().active_movement =
            ActiveMovement::new(movement_sequence, 0);
        entity.position_iface_mut().set_map_goal(goal);
    }

    let sibling = SequenceElement::new(1, Command::SpeakHeroReachDestination, Some(owner));
    let sibling_sequence = engine.orders.sequence_manager.launch_element(sibling);
    // The PC speech override terminates before delegating to Actor::Instruct,
    // so it never replaces the selected movement pointer.
    engine
        .orders
        .sequence_manager
        .element_terminated(sibling_sequence, 0);
    engine.dispatch_condolations(&sim, &LevelAssets::new());

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

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::SimulatingBeggar));

    let mut leave_beggar = SequenceElement::new(1, Command::LeaveBeggar, Some(owner));
    leave_beggar.priority = SequencePriority::NonInterruptable;
    let blocker = engine.orders.sequence_manager.launch_element(leave_beggar);
    engine
        .orders
        .sequence_manager
        .element_in_progress(blocker, 0);

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
    let movement = engine.launch_sequence(sequence);

    assert!(engine.non_interruptable_guard(owner, movement, 0));
    assert!(engine.non_interruptable_guard(owner, movement, 1));

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
    use crate::movement::ActiveMovement;
    use crate::order::OrderType;
    use crate::sequence::{CascadeFlags, SequenceElement};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    let goal = MapPoint::new(1004.836, 1774.2802);

    let movement =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    let movement_seq = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_seq, 0);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.actor_data_mut().unwrap().active_movement = ActiveMovement::new(movement_seq, 0);
        entity.position_iface_mut().set_map_goal(goal);
    }

    let incoming_seq = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(
            1,
            Command::EnterAttentiveMode,
            Some(owner),
        ));
    engine
        .orders
        .sequence_manager
        .begin_instruct_callback(owner, incoming_seq, 0);
    engine
        .orders
        .sequence_manager
        .element_interrupted(movement_seq, 0, CascadeFlags::NEXT_LEVEL);
    engine.dispatch_condolations(&sim, &LevelAssets::new());
    engine
        .orders
        .sequence_manager
        .end_instruct_callback(owner, incoming_seq, 0);

    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(
        entity.actor_data().unwrap().active_movement,
        ActiveMovement::none(),
        "Rust's stale movement tracker must still detach"
    );
    assert_eq!(
        entity.position_iface().map_goal(),
        goal,
        "Original clears the sprite goal only when the outgoing movement is still selected"
    );
}

#[test]
fn halt_condolation_does_not_instruct_a_prequeued_replacement_move() {
    use crate::element::{Command, Posture};
    use crate::order::{AiOrderIntent, OrderType};
    use crate::sequence::{CascadeFlags, SequenceElement};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    let outgoing =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    let outgoing_seq = engine.orders.sequence_manager.launch_element(outgoing);
    engine
        .orders
        .sequence_manager
        .element_in_progress(outgoing_seq, 0);

    engine.orders.pending_move_requests.push((
        owner,
        AiOrderIntent::new(OrderType::WalkingUpright, 90.0, 40.0),
    ));
    assert_eq!(engine.orders.pending_move_requests.len(), 1);

    engine.orders.sequence_manager.set_halt_pending(true);
    engine
        .orders
        .sequence_manager
        .element_interrupted(outgoing_seq, 0, CascadeFlags::NEXT_LEVEL);
    engine.orders.sequence_manager.set_halt_pending(false);
    engine.dispatch_condolations(&sim, &LevelAssets::new());

    assert_eq!(
        engine.orders.pending_move_requests.len(),
        1,
        "a Halt card suppresses Think and must not steal its caller's replacement Move"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .all(|sequence| sequence.id == outgoing_seq),
        "the replacement must remain unregistered until its normal owner/manager boundary"
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
    let soldier = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    set_test_soldier_brawl_got_hit(&mut engine, soldier);

    let mut parent = Sequence::new();
    parent.append_element(SequenceElement::new(1, Command::LookLeft, Some(soldier)));
    // IsLastRealAction explicitly skips Wait/AssertPosition successors,
    // so the LookLeft condolence still fires before Ready queues this.
    parent.append_element(SequenceElement::new(2, Command::Wait, Some(soldier)));
    let parent_id = engine.orders.sequence_manager.launch_sequence(parent);

    let initial = engine.orders.sequence_manager.hourglass();
    assert_eq!(initial.len(), 1);
    engine
        .orders
        .sequence_manager
        .element_in_progress(parent_id, 0);
    engine
        .orders
        .sequence_manager
        .element_terminated(parent_id, 0);
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.dispatch_condolations(sim, &assets);

    let commands: Vec<_> = engine
        .orders
        .sequence_manager
        .hourglass()
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
        "SendCondolationCard's recursive Think must launch/arbitrate its action before Ready queues the parent's next level"
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

    let mut engine = EngineInner::new();
    let first = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    let second = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    let third = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
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
    let seq_id = engine.orders.sequence_manager.launch_sequence(seq);

    engine
        .orders
        .sequence_manager
        .element_interrupted(seq_id, 0, CascadeFlags::NEXT_LEVEL);
    engine.dispatch_condolations_for_npc(sim, first, &LevelAssets::new());

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
            "cross-owner card {idx} must run inside the originating SetState cascade"
        );
    }
    assert!(
        engine
            .orders
            .sequence_manager
            .drain_pending_condolations()
            .is_empty()
    );
}

#[test]
fn condolation_ready_executes_immediate_timer_successor_inline() {
    use crate::element::Command;
    use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};

    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));

    // The mission regression had three actor elements at the current command
    // level and an ownerless Timer at index 3.  The last actor condolence
    // resumes Ready(), which must execute that Timer before SetState returns.
    let mut sequence = Sequence::new();
    for command in [Command::LookLeft, Command::LookRight, Command::LookLeft] {
        sequence.append_element(SequenceElement::new(1, command, Some(owner)));
    }
    let mut timer = SequenceElement::new_generic(2, Command::Timer, None);
    timer.set_property(Field::Timer, FieldValue::Integer(12));
    sequence.append_element(timer);
    let sequence_id = engine.orders.sequence_manager.launch_sequence(sequence);
    let initial = engine.orders.sequence_manager.hourglass();
    assert_eq!(initial.len(), 3);

    // Suppress the AI EventDone callbacks just as Halt does; the regression is
    // the continuation of SetState after SendCondolationCard returns.
    engine.orders.sequence_manager.set_halt_pending(true);
    for element_index in 0..3 {
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, element_index);
        engine
            .orders
            .sequence_manager
            .element_terminated(sequence_id, element_index);
    }
    engine.orders.sequence_manager.set_halt_pending(false);

    engine.dispatch_condolations(sim, &LevelAssets::new());

    assert_eq!(engine.orders.timer_elements.len(), 1);
    assert_eq!(engine.orders.timer_elements[0].remaining, 12);
    assert!(
        engine
            .orders
            .sequence_manager
            .take_pending_synchronous_actions()
            .is_empty(),
        "Ready's immediate successor must not escape the condolence boundary"
    );
}
