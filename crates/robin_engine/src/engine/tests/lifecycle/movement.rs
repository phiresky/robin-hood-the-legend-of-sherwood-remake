use super::*;

#[test]
fn terminal_building_move_preserves_prior_actor_done_edge() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::position_interface::SectorHandle;
    use crate::sequence::{SequenceElement, SequenceElementData, SequenceState};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    install_test_building_sector(&mut engine, 42);
    let interior_plane = crate::position_interface::PlaneZCoeffs {
        az: 0.0,
        bz: 0.0,
        dz: 75.0,
    };
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.position_iface_mut().set_sector_topology(
            SectorHandle::new(42),
            crate::fast_find_grid::SectorIndex::new(0),
        );
        entity
            .position_iface_mut()
            .set_obstacle(None, Some(interior_plane));
        entity.actor_data_mut().unwrap().continuation.motion_state = MotionState::Done;
    }

    let mut movement =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    let SequenceElementData::Movement { destination, .. } = &mut movement.data else {
        unreachable!("new_movement must produce movement data")
    };
    *destination = crate::coordinates::MapPoint::new(100.0, 200.0);
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);

    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut HostDisplayState::default(),
        &LevelAssets::default(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(movement_sequence, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(entity.position_iface().get_plane(), Some(&interior_plane));
    assert_eq!(entity.element_data().position().z, 75.0);
    assert_eq!(
        entity.element_data().position_map(),
        crate::coordinates::MapPoint::new(100.0, 200.0)
    );
    let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
    assert_eq!(actor.installed_order, None);
    assert_eq!(
        actor.continuation.motion_state,
        MotionState::Done,
        "translation-time selection loss returns before actor instruction stamps InProgress"
    );
}

#[test]
fn hourglass_phase_trace_locks_entity_npc_path_sequence_and_deferred_order() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();

    begin_hourglass_phase_capture();
    let result = engine
        .perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev)
        .code;
    let phases = end_hourglass_phase_capture();

    assert_eq!(result, GameCode::LevelInProgress);
    assert_eq!(
        phases,
        vec![
            HourglassPhase::DeferredEffectsStart,
            HourglassPhase::MissionAndMessages,
            HourglassPhase::NpcOrders,
            HourglassPhase::Paths,
            HourglassPhase::Entities,
            HourglassPhase::EntitySystems,
            HourglassPhase::Npcs,
            HourglassPhase::GameplaySystems,
            HourglassPhase::Sequences,
            HourglassPhase::DeferredEffectsEnd,
        ]
    );
}

#[test]
fn hourglass_phase_trace_stops_after_the_locked_mission_gate() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.set_engine_locked(true);
    let pending_owner = EntityId::Pc(crate::entity_id::PcId(319));
    let pending_sequence = engine.orders.sequence_manager.launch_element(
        crate::sequence::SequenceElement::new_movement(
            1,
            crate::element::Command::AssertPosition,
            Some(pending_owner),
            crate::order::OrderType::WalkingUpright,
        ),
    );
    engine
        .orders
        .pending_hades_kills
        .push(EntityId::new(99, crate::element::EntityIdKind::Soldier));

    begin_hourglass_phase_capture();
    let result = engine
        .perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev)
        .code;
    let phases = end_hourglass_phase_capture();

    assert_eq!(result, GameCode::LevelInProgress);
    assert_eq!(
        engine.control.frame_counter, 1,
        "the lock gate follows clock advance"
    );
    assert!(
        engine.orders.pending_hades_kills.is_empty(),
        "deferred order work must drain before the locked mission gate"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(pending_sequence, 0)
            .expect("locked gate must retain the registered sequence element")
            .state,
        crate::sequence::SequenceState::Todo,
        "the locked mission gate must leave manager-FIFO work for a later hourglass"
    );
    assert_eq!(
        phases,
        vec![
            HourglassPhase::DeferredEffectsStart,
            HourglassPhase::MissionAndMessages,
        ]
    );
}

#[test]
fn move_ok_bored_exit_transition_uses_generic_actor_execute() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::Bored;

    let transition = OrderType::TransitionWaitingUprightBoredWaitingUpright;
    let script = SpriteScript {
        action_id: transition as u16,
        action_done: 2,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![2; 3],
        distances: vec![0; 3],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[transition as usize] = 0;
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(conversion),
    );

    let mut selected =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::WalkingUpright);
    selected
        .orders
        .push_back(Order::test_new(transition, 0.0, 0.0));
    let sequence = engine.orders.sequence_manager.launch_element(selected);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    let mut executed_transition = false;
    for _ in 0..16 {
        let (_, _, executed) = engine.tick_actor_animation_for(&sim, &assets, owner);
        executed_transition |= executed.is_some();
        if engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .action_state
            == ActionState::Waiting
        {
            break;
        }
    }
    assert!(
        executed_transition,
        "a transition selected as GenericAnimation must not be suppressed merely because its element carries Movement data"
    );
    assert_eq!(
        engine.get_entity(owner).unwrap().sprite().last_action,
        transition
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .action_state,
        ActionState::Waiting,
        "bored-exit completion inside MoveOk must apply the base-Actor state transition"
    );
}

#[test]
fn deferred_face_to_does_not_overwrite_a_newer_live_movement_goal() {
    use crate::coordinates::MapPoint;
    use crate::element::{ActionState, Posture};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::Moving;

    let stale_retained_goal = MapPoint::new(70.0, 80.0);
    let live_goal = MapPoint::new(90.0, 100.0);
    engine.launch_turn_sequence_deferred_no_transitions(
        owner,
        crate::element::Command::Turn,
        Some(9),
        0.0,
        0.0,
        Some(stale_retained_goal),
    );

    // The outgoing actor slot may run after facing registers its deferred
    // Turn and advance the movement goal before SequenceManager instructs it.
    engine
        .get_entity_mut(owner)
        .unwrap()
        .position_iface_mut()
        .set_map_goal(live_goal);
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        live_goal,
        "deferred Turn instruction must not replace a goal advanced by the outgoing actor slot"
    );
}

#[test]
fn positional_face_to_captures_direction_before_deferred_manager_instruction() {
    use crate::coordinates::MapPoint;
    use crate::element::{Command, Posture};
    use crate::order::AiOrderIntent;
    use crate::sequence::{Field, FieldValue, SequenceState};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    let mut soldier = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier_data) = &mut soldier else {
        unreachable!("make_test_soldier returned a non-soldier")
    };
    soldier_data.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    soldier
        .element_data_mut()
        .set_position_map(MapPoint::new(100.0, 100.0));
    let owner = engine.add_entity(soldier);
    let target = MapPoint::new(200.0, 100.0);
    let expected_direction =
        crate::position_interface::vector_to_sector_0_to_15_iso(target.x - 100.0, target.y - 100.0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .outbox
        .actor
        .orders
        .push(AiOrderIntent::face_toward(target.x, target.y));

    engine.launch_pending_orders_for_npc(&sim, &assets, owner);

    let turn_sequence = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .find(|sequence| {
            sequence.elements.first().is_some_and(|element| {
                element.owner == Some(owner) && element.command == Command::Turn
            })
        })
        .expect("positional facing registered a deferred turn");
    let turn = &turn_sequence.elements[0];
    assert_eq!(turn.state, SequenceState::Todo);
    assert!(turn.orders.is_empty());
    assert!(matches!(
        turn.get_property(Field::Direction),
        Some(FieldValue::Integer(direction)) if *direction == expected_direction as u32
    ));
    assert!(turn.get_property(Field::CameraPoint).is_none());
    let turn_sequence_id = turn_sequence.id;

    // If manager-time instruction incorrectly re-resolves the point, this
    // position would reverse the requested direction.
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .set_position_map(MapPoint::new(300.0, 100.0));
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    assert_eq!(
        u8::from(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .get_direction_goal()
        ),
        expected_direction as u8
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(turn_sequence_id, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );
}

#[test]
fn goto_replacement_retains_selected_movement_goal_while_path_is_pending() {
    use crate::coordinates::MapPoint;
    use crate::element::{ActionState, Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{Order, OrderType};
    use crate::sequence::{CascadeFlags, SequenceElement, SequencePriority};
    use std::num::NonZeroU32;

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let mut soldier = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier_data) = &mut soldier else {
        unreachable!("make_test_soldier returned a non-soldier")
    };
    soldier_data.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    let owner = engine.add_entity(soldier);

    let old_goal = MapPoint::new(70.0, 80.0);
    let mut movement =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::RunningUpright);
    movement.priority = SequencePriority::Normal;
    movement.orders.push_back(Order::new(
        OrderType::RunningUpright,
        old_goal.x,
        old_goal.y,
        NonZeroU32::new(778).unwrap(),
    ));
    let old_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(old_sequence, 0);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.actor_data_mut().unwrap().action_state = ActionState::MovingFast;
        entity.actor_data_mut().unwrap().active_movement = ActiveMovement::new(old_sequence, 0);
        entity.position_iface_mut().set_map_goal(old_goal);
    }

    let mut replacement = SequenceElement::new_movement(
        1,
        Command::MoveWaiting,
        Some(owner),
        OrderType::RunningUpright,
    );
    replacement.priority = SequencePriority::Normal;
    replacement.retained_movement_goal = Some(old_goal);
    let replacement_sequence = engine.orders.sequence_manager.launch_element(replacement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(replacement_sequence, 0);
    engine.orders.sequence_manager.set_halt_pending(true);
    engine
        .orders
        .sequence_manager
        .element_interrupted_after_replacement_selected(old_sequence, 0, CascadeFlags::NEXT_LEVEL);
    engine.orders.sequence_manager.set_halt_pending(false);
    engine.dispatch_condolations(&sim, &assets);

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        old_goal,
        "replacement selection must happen before the old movement's condolence can clear its cached transition goal"
    );
}

#[test]
fn goto_replacing_move_waiting_publishes_gate_failure_before_tail_halt() {
    use crate::element::{Command, Posture};
    use crate::order::{AiOrderIntent, OrderType};
    use crate::position_interface::SectorHandle;
    use crate::sequence::{SequenceElement, SequencePriority};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let mut soldier = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier_data) = &mut soldier else {
        unreachable!("make_test_soldier returned a non-soldier")
    };
    soldier_data.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    let owner = engine.add_entity(soldier);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .set_sector(SectorHandle::new(1));

    let mut waiting = SequenceElement::new_movement(
        1,
        Command::MoveWaiting,
        Some(owner),
        OrderType::RunningUpright,
    );
    waiting.priority = SequencePriority::Normal;
    let waiting_sequence = engine.orders.sequence_manager.launch_element(waiting);
    engine
        .orders
        .sequence_manager
        .element_in_progress(waiting_sequence, 0);

    let mut intent = AiOrderIntent::new(OrderType::RunningUpright, 100.0, 200.0);
    intent.target_sector = SectorHandle::new(119);
    intent.target_layer = Some(8);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .outbox
        .reentrant
        .reconsider_approach_completion_pending = true;
    engine
        .get_entity_mut(owner)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .outbox
        .actor
        .orders
        .push(intent);

    engine.launch_pending_orders_for_npc(&sim, &assets, owner);

    let ai = engine.get_entity(owner).unwrap().ai_controller().unwrap();
    assert!(
        ai.couldnt_reachpoint,
        "movement construction's synchronous gate failure must reach decision completion"
    );
    assert!(
        ai.outbox.reentrant.reconsider_approach_replaced_path_waiter,
        "the typed reconsider continuation must retain that its failed route replaced MoveWaiting"
    );
    assert!(engine.orders.pending_move_requests.is_empty());
}

#[test]
fn goto_replacing_move_waiting_constructs_authorized_move_before_tail_halt() {
    use crate::element::{Command, Posture};
    use crate::order::{AiOrderIntent, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let mut soldier = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier_data) = &mut soldier else {
        unreachable!("make_test_soldier returned a non-soldier")
    };
    soldier_data.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    let owner = engine.add_entity(soldier);

    let mut waiting = SequenceElement::new_movement(
        1,
        Command::MoveWaiting,
        Some(owner),
        OrderType::RunningUpright,
    );
    waiting.priority = SequencePriority::Normal;
    let waiting_sequence = engine.orders.sequence_manager.launch_element(waiting);
    engine
        .orders
        .sequence_manager
        .element_in_progress(waiting_sequence, 0);

    engine
        .get_entity_mut(owner)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .outbox
        .actor
        .orders
        .push(AiOrderIntent::new(OrderType::RunningUpright, 100.0, 200.0));

    engine.launch_pending_orders_for_npc(&sim, &assets, owner);

    assert_eq!(engine.orders.pending_move_requests.len(), 1);
    assert!(
        engine.orders.pending_move_requests[0]
            .1
            .halt_after_launch_for_path_waiter,
        "movement's tail halt must remain attached until the replacement sequence has been constructed"
    );

    let sequence_count_before_drain = engine.orders.sequence_manager.sequence_count();
    engine.drain_pending_move_requests(&sim);

    assert!(engine.orders.pending_move_requests.is_empty());
    assert!(
        engine.orders.sequence_manager.sequence_count() > sequence_count_before_drain,
        "the replacement sequence must be constructed before movement applies its tail halt"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .pending_elements_for_owner(owner)
            .iter()
            .all(|(sequence_id, _)| *sequence_id == waiting_sequence),
        "the post-construction movement tail must cancel the replacement before instruction; only the old waiter's stop transition may remain"
    );
}

#[test]
fn deferred_ai_move_skips_recursive_owner_drain_then_promotes_globally() {
    use crate::element::Posture;
    use crate::order::{AiOrderIntent, OrderType};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut soldier = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier_data) = &mut soldier else {
        unreachable!("make_test_soldier returned a non-soldier")
    };
    soldier_data.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    let owner = engine.add_entity(soldier);

    let mut intent = AiOrderIntent::new(OrderType::RunningUpright, 100.0, 200.0);
    intent.defer_instruction = true;
    engine.launch_ai_move(owner, &intent);

    assert!(
        engine
            .drain_pending_move_requests_for_owner(&sim, owner)
            .is_empty(),
        "recursive owner closure must not promote the deferred roof move"
    );
    assert_eq!(engine.orders.pending_move_requests.len(), 1);

    engine.drain_pending_move_requests(&sim);
    assert_eq!(
        engine.orders.pending_move_requests.len(),
        1,
        "the later global drain in the authored frame must also retain the move"
    );
    engine.control.frame_counter += 1;
    engine.drain_pending_move_requests(&sim);
    assert!(engine.orders.pending_move_requests.is_empty());
    assert!(
        engine
            .orders
            .sequence_manager
            .deferred_elements_to_go()
            .iter()
            .any(|(sequence_id, element_index)| engine
                .orders
                .sequence_manager
                .get_element(*sequence_id, *element_index)
                .is_some_and(|element| element.owner == Some(owner)))
    );
}

#[test]
fn path_waiter_tail_halts_registered_roof_move_before_instruction() {
    use crate::element::{Command, Posture};
    use crate::order::{AiOrderIntent, OrderType};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut soldier = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier_data) = &mut soldier else {
        unreachable!("make_test_soldier returned a non-soldier")
    };
    soldier_data.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    let owner = engine.add_entity(soldier);

    let mut intent = AiOrderIntent::new(OrderType::RunningUpright, 100.0, 200.0);
    intent.halt_after_launch_for_path_waiter = true;
    engine.launch_ai_move(owner, &intent);

    let launched = engine.drain_pending_move_requests_for_owner(&sim, owner);
    assert!(
        launched.is_empty(),
        "the path-waiter tail must not expose the interrupted sequence for instruction"
    );
    assert!(engine.orders.pending_move_requests.is_empty());
    assert!(
        engine
            .orders
            .sequence_manager
            .pending_elements_for_owner(owner)
            .is_empty(),
        "movement's post-launch halt removes the roof Move from the manager FIFO"
    );
    assert_eq!(engine.actor_command(owner), Command::Wait);
}

#[test]
fn fallback_staging_preserves_authored_path_waiter_tail_after_waiter_is_gone() {
    use crate::element::Posture;
    use crate::order::{AiOrderIntent, OrderType};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let mut soldier = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier_data) = &mut soldier else {
        unreachable!("make_test_soldier returned a non-soldier")
    };
    soldier_data.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    let owner = engine.add_entity(soldier);

    let mut fallback = AiOrderIntent::new(OrderType::RunningUpright, 100.0, 200.0);
    fallback.halt_after_launch_for_path_waiter = true;
    engine
        .get_entity_mut(owner)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .outbox
        .actor
        .orders
        .push(fallback);

    engine.launch_pending_orders_for_npc(&sim, &assets, owner);

    assert_eq!(engine.orders.pending_move_requests.len(), 1);
    assert!(
        engine.orders.pending_move_requests[0]
            .1
            .halt_after_launch_for_path_waiter,
        "roof-fallback staging must not erase the path-waiter tail after the outgoing waiter was halted"
    );
}

#[test]
fn ordinary_move_staging_does_not_invent_path_waiter_tail() {
    use crate::element::Posture;
    use crate::order::{AiOrderIntent, OrderType};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let mut soldier = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier_data) = &mut soldier else {
        unreachable!("make_test_soldier returned a non-soldier")
    };
    soldier_data.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    let owner = engine.add_entity(soldier);

    engine
        .get_entity_mut(owner)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .outbox
        .actor
        .orders
        .push(AiOrderIntent::new(OrderType::RunningUpright, 100.0, 200.0));

    engine.launch_pending_orders_for_npc(&sim, &assets, owner);

    assert_eq!(engine.orders.pending_move_requests.len(), 1);
    assert!(
        !engine.orders.pending_move_requests[0]
            .1
            .halt_after_launch_for_path_waiter,
        "ordinary movement without a live or inherited path waiter must remain unmarked"
    );
}

#[test]
fn ration_set_path_updates_eat_or_guzzle_slot_without_out_of_ammo_speech() {
    use crate::campaign::PcDescription;
    use crate::profiles::{Action, CharacterProfile, CharacterProfileIdx};

    for (action, starting_ammo) in [
        (Action::Eat, 1),
        (Action::Guzzle, 1),
        (Action::Eat, 2),
        (Action::Guzzle, 2),
    ] {
        let mut engine = EngineInner::new();
        let mut pc = make_test_pc(crate::element::Posture::Upright);
        let pc_data = pc.pc_data_mut().unwrap();
        pc_data.profile_index = CharacterProfileIdx(0);
        pc_data.campaign_description_index = Some(0);
        pc_data.current_action = action;
        pc_data.saved_action = action;
        pc_data.disabled_actions = vec![false; 3];
        pc_data.disabled_actions_temp = vec![false; 3];
        pc_data.disabled_actions[0] = true;
        let pc_id = engine.add_entity(pc);

        let mut desc = PcDescription {
            character_profile_idx: Some(CharacterProfileIdx(0)),
            ..Default::default()
        };
        desc.status.set_ammo(action, starting_ammo);
        engine.mission_domain.campaign.characters.push(desc);

        let mut assets = LevelAssets::new();
        std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .push(CharacterProfile {
                actions: [action, Action::NoAction, Action::NoAction],
                ..Default::default()
            });

        engine.consume_ration_without_speech(&assets, pc_id, action);

        assert_eq!(
            engine.mission_domain.campaign.characters[0]
                .status
                .get_ammo(action),
            starting_ammo - 1
        );
        let pc = engine.get_entity(pc_id).unwrap().pc_data().unwrap();
        if starting_ammo == 1 {
            assert_eq!(pc.current_action, Action::NoAction);
            assert_eq!(pc.saved_action, Action::NoAction);
            assert!(pc.disabled_actions[0]);
        } else {
            assert_eq!(pc.current_action, action);
            assert_eq!(pc.saved_action, action);
            assert!(!pc.disabled_actions[0]);
        }
        assert!(engine.feedback.sound_sim.pending_exclamations.is_empty());
    }
}
