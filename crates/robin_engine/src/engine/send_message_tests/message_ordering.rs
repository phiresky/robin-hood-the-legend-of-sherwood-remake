use super::*;

fn send_message_element(
    level: u16,
    owner: Option<crate::element::EntityId>,
    message: i32,
) -> SequenceElement {
    let mut element = SequenceElement::new_generic(level, Command::SendMessage, owner);
    element.set_property(Field::Message, FieldValue::Integer(message as u32));
    element.set_property(Field::MessageArgument, FieldValue::Integer(0));
    element.set_property(Field::MessageExtendedArgument, FieldValue::Integer(0));
    element
}

#[test]
fn recorded_lock_ai_stops_old_animation_before_its_unlock_and_starts_new_animation() {
    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let (mut engine, receiver, _) = engine_with_receiver();
    let mut display = crate::engine::HostDisplayState::default();

    engine
        .get_entity_mut(receiver)
        .expect("receiver")
        .ai_controller_mut()
        .expect("receiver NPC AI")
        .script_locked = true;

    let mut old_sequence = Sequence::new();
    let mut old_animation = SequenceElement::new_generic(1, Command::PlayAnim, Some(receiver));
    old_animation.set_property(
        Field::AnimationId,
        FieldValue::Animation(OrderType::TransitionLoweringSword),
    );
    old_sequence.append_element(old_animation);
    old_sequence.append_element(SequenceElement::new_generic(
        2,
        Command::UnlockAi,
        Some(receiver),
    ));
    let old_id = engine.launch_sequence(old_sequence);
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(receiver),
        Some((old_id, 0)),
        "the old animation must be live before the replacement sequence arrives"
    );

    let mut replacement = Sequence::new();
    replacement.append_element(SequenceElement::new_generic(
        1,
        Command::LockAi,
        Some(receiver),
    ));
    let mut new_animation = SequenceElement::new_generic(2, Command::PlayAnim, Some(receiver));
    new_animation.set_property(
        Field::AnimationId,
        FieldValue::Animation(OrderType::RaisingShield),
    );
    replacement.append_element(new_animation);
    let replacement_id = engine.launch_sequence(replacement);
    engine.drain_pending_immediate_actions_sync(&sim, &mut display, &assets);
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let manager = &engine.orders.sequence_manager;
    assert_eq!(
        manager
            .get_element(old_id, 0)
            .expect("old animation remains inspectable")
            .state,
        SequenceState::Interrupted
    );
    assert_eq!(
        manager
            .get_element(old_id, 1)
            .expect("old unlock remains inspectable")
            .state,
        SequenceState::Interrupted,
        "stopping the old animation must cascade across its trailing UnlockAi"
    );
    assert_eq!(
        manager
            .get_element(replacement_id, 0)
            .expect("replacement lock remains inspectable")
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        manager
            .get_element(replacement_id, 1)
            .expect("replacement animation remains inspectable")
            .state,
        SequenceState::InProgress
    );
    assert_eq!(
        manager.current_element_for_actor(receiver),
        Some((replacement_id, 1)),
        "the new PlayAnim must become the actor's live command"
    );

    let ai = engine
        .get_entity(receiver)
        .expect("receiver")
        .ai_controller()
        .expect("receiver NPC AI");
    assert!(ai.script_locked, "the replacement lock must remain held");
    assert!(
        !ai.outbox
            .reentrant
            .self_stimuli
            .iter()
            .any(|queued| queued.stimulus_type == crate::ai::StimulusType::EventReturnToDuty),
        "the interrupted old UnlockAi must not schedule ReturnToDuty"
    );
}

#[test]
fn script_send_message_sequence_does_not_preempt_current_actor_element() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let (mut engine, receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    let active_id = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new_movement(
            1,
            Command::Move,
            Some(receiver),
            OrderType::RunningUpright,
        ));
    engine
        .orders
        .sequence_manager
        .element_in_progress(active_id, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(receiver),
        Some((active_id, 0))
    );

    let frame_before = engine.control.frame_counter;
    engine
        .call_external_native(
            sim,
            &assets,
            "SendMessageWithArguments",
            &[handle, 1234, 55, -7],
        )
        .expect("SendMessageWithArguments should complete synchronously");

    assert_eq!(
        engine.control.frame_counter, frame_before,
        "SendMessage is zero-frame"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(receiver),
        Some((active_id, 0)),
        "ExecutedImmediately bypasses Instruct contention and preserves the current element"
    );

    let send = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .find(|element| element.command == Command::SendMessage)
        .expect("native request should launch a SendMessage sequence element");
    assert_eq!(send.owner, Some(receiver));
    assert_eq!(send.state, SequenceState::Terminated);
    assert_eq!(integer_property(send, Field::Message), 1234);
    assert_eq!(integer_property(send, Field::MessageArgument), 55);
    assert_eq!(
        integer_property(send, Field::MessageExtendedArgument),
        (-7_i32) as u32
    );
}

#[test]
fn script_send_message_callback_completes_before_sequence_launch_returns() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let (mut engine, _receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    // Original: RHScript::SendMessage calls LaunchSequenceElement, whose
    // RHCOMMAND_SEND_MESSAGE ExecutedImmediately path invokes ProcessMessage
    // inline (RHScript.cpp:6846-6865; RHsequenceelement.cpp:736-777).
    engine
        .call_external_native(sim, &assets, "SendMessage", &[handle, 314])
        .expect("SendMessage should complete synchronously");

    assert_eq!(
        engine
            .scripts
            .mission
            .as_ref()
            .expect("script installed")
            .state
            .globals
            .get(&900),
        Some(&314),
        "the nested ProcessMessage mutation must be visible when sequence launch returns"
    );
    let send = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .find(|element| element.command == Command::SendMessage)
        .expect("SendMessage launch should retain its sequence element");
    assert_eq!(
        send.state,
        SequenceState::Terminated,
        "ProcessMessage and termination both happen inside the launch call"
    );
}

#[test]
fn script_send_message_callbacks_run_in_launch_order_in_same_frame() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let (mut engine, _receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let frame_before = engine.control.frame_counter;

    engine
        .call_external_native(sim, &assets, "SendMessage", &[handle, 41])
        .expect("first SendMessage");
    engine
        .call_external_native(sim, &assets, "SendMessage", &[handle, 72])
        .expect("second SendMessage");

    assert_eq!(engine.control.frame_counter, frame_before);
    assert_eq!(
        engine
            .scripts
            .mission
            .as_ref()
            .expect("script installed")
            .state
            .globals
            .get(&900),
        Some(&72),
        "ProcessMessage callbacks must run in SendMessage launch order"
    );
    let states: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| element.command == Command::SendMessage)
        .map(|element| element.state)
        .collect();
    assert_eq!(
        states,
        vec![SequenceState::Terminated, SequenceState::Terminated],
        "both callbacks and terminations complete without advancing a frame"
    );
}

#[test]
fn registered_send_message_callback_precedes_later_immediate_sibling() {
    let (mut engine, _, _) = engine_with_receiver();
    let receiver = engine.add_entity(scripted_soldier("OrderingReceiver"));
    let handle = bind_script_actor(&mut engine, receiver, "OrderingReceiver");
    engine
        .get_entity_mut(receiver)
        .expect("receiver")
        .element_data_mut()
        .blipped = true;

    let mut sequence = Sequence::new();
    sequence.append_element(send_message_element(1, Some(receiver), 77));
    sequence.append_element(SequenceElement::new_generic(
        1,
        Command::Unblip,
        Some(receiver),
    ));
    engine.orders.sequence_manager.launch_sequence(sequence);

    let mut display = crate::engine::HostDisplayState::default();
    engine.drain_pending_immediate_actions_sync(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::new(),
    );

    assert_eq!(
        engine
            .scripts
            .mission
            .as_ref()
            .expect("script installed")
            .state
            .globals
            .get(&904),
        Some(&0),
        "ProcessMessage must observe state before the later Unblip sibling"
    );
    assert_eq!(
        engine
            .get_entity(receiver)
            .expect("receiver")
            .element_data()
            .blipped,
        false
    );
    assert_eq!(ScriptHandleCodec::actor_handle(receiver), handle);
}

#[test]
fn target_activation_callback_precedes_later_engine_sibling() {
    let (mut engine, reader, _) = engine_with_receiver();
    let target = engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: ElementData {
            kind: ElementKind::Target,
            active: true,
            ..Default::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let target_handle = ScriptHandleCodec::actor_handle(target);
    let target_instance = engine
        .scripts
        .mission
        .as_ref()
        .expect("script installed")
        .manager
        .create_instance("TargetOrdering")
        .expect("target callback class");
    engine
        .scripts
        .mission
        .as_mut()
        .expect("script installed")
        .target_instances
        .insert(target_handle, target_instance);

    let mut sequence = Sequence::new();
    sequence.append_element(SequenceElement::new_interaction(
        1,
        Command::ActivateArrow,
        Some(target),
        Some(reader),
    ));
    let mut unfreeze = SequenceElement::new_generic(1, Command::FreezeAll, None);
    unfreeze.set_property(Field::Freeze, FieldValue::Bool(false));
    sequence.append_element(unfreeze);
    engine.orders.sequence_manager.launch_sequence(sequence);

    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::new(),
    );

    assert!(
        !engine.actors_frozen(),
        "ActivatedByArrow's callback must run before the later unfreeze sibling"
    );
}

#[test]
fn send_message_callback_precedes_later_move_translation() {
    let (mut engine, _, _) = engine_with_receiver();
    let mover = engine.add_entity(scripted_soldier("MoveOrdering"));
    let mover_handle = bind_script_actor(&mut engine, mover, "MoveOrdering");
    engine
        .get_entity_mut(mover)
        .expect("mover")
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::new(0.0, 0.0));
    engine
        .get_entity_mut(mover)
        .expect("mover")
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_coords(
            -4.0, -4.0, 4.0, 4.0,
        ));

    let mut movement =
        SequenceElement::new_movement(1, Command::Move, Some(mover), OrderType::WalkingUpright);
    if let crate::sequence::SequenceElementData::Movement { destination, .. } = &mut movement.data {
        *destination = crate::coordinates::MapPoint::new(20.0, 0.0);
    }
    let mut sequence = Sequence::new();
    sequence.append_element(send_message_element(1, Some(mover), 79));
    sequence.append_element(movement);
    let sequence_id = engine.orders.sequence_manager.launch_sequence(sequence);

    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut crate::engine::HostDisplayState::default(),
        &assets,
    );

    assert_eq!(
        engine
            .scripts
            .mission
            .as_ref()
            .expect("script installed")
            .state
            .globals
            .get(&909),
        Some(&(crate::order::OrderType::NonanimationEnd as i32)),
        "ProcessMessage must observe the no-installed-order sentinel before the later FIFO Move is translated"
    );
    assert_ne!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 1)
            .expect("Move element")
            .state,
        SequenceState::Todo,
        "the later Move still dispatches in the same hourglass"
    );
    assert_eq!(ScriptHandleCodec::actor_handle(mover), mover_handle);
}

#[test]
fn ownerless_message_runs_wait_successor_before_older_immediate_sibling() {
    let (mut engine, receiver, _) = engine_with_receiver();
    engine
        .get_entity_mut(receiver)
        .expect("receiver")
        .element_data_mut()
        .blipped = true;

    let mut message_then_wait = Sequence::new();
    message_then_wait.append_element(send_message_element(1, None, 80));
    let mut wait = SequenceElement::new(2, Command::Wait, Some(receiver));
    wait.priority = crate::sequence::SequencePriority::Wait;
    message_then_wait.append_element(wait);
    let sequence_id = engine
        .orders
        .sequence_manager
        .launch_sequence(message_then_wait);

    let mut older_sibling = Sequence::new();
    older_sibling.append_element(SequenceElement::new_generic(
        1,
        Command::Unblip,
        Some(receiver),
    ));
    engine
        .orders
        .sequence_manager
        .launch_sequence(older_sibling);

    engine.drain_pending_immediate_actions_sync(
        &crate::sim_rng::test_context(),
        &mut crate::engine::HostDisplayState::default(),
        &LevelAssets::new(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 1)
            .expect("WAIT successor")
            .state,
        SequenceState::InProgress,
        "Ready() must run the WAIT successor before returning to the detached sibling"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .next_pending_immediate_action()
            .is_none(),
        "the complete synchronous stream must be drained"
    );
    assert!(
        !engine
            .get_entity(receiver)
            .expect("receiver")
            .element_data()
            .blipped,
        "the older immediate sibling runs after the WAIT successor"
    );
}

#[test]
fn recorded_actor_message_closes_ready_before_parent_vm_resumes() {
    let (mut engine, _, _) = engine_with_receiver();
    let actor = engine.add_entity(scripted_soldier("OrderingReceiver"));
    let handle = bind_script_actor(&mut engine, actor, "OrderingReceiver");
    engine
        .get_entity_mut(actor)
        .expect("actor")
        .element_data_mut()
        .blipped = true;

    engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            super::ScriptVmKey::Actor(handle),
            "TriggerNextLevel",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("recorded SendMessage successor should finish before Thanx resumes");

    assert_eq!(
        engine
            .scripts
            .mission
            .as_ref()
            .expect("script installed")
            .state
            .globals
            .get(&908),
        Some(&1),
        "the parent VM must observe the next-level Unblip successor"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .next_pending_immediate_action()
            .is_none()
    );
}

#[test]
fn missing_send_message_receiver_vm_terminates_and_runs_successor() {
    let (mut engine, _, _) = engine_with_receiver();
    let receiver = engine.add_entity(scripted_soldier(""));
    engine
        .get_entity_mut(receiver)
        .expect("receiver")
        .element_data_mut()
        .blipped = true;

    let mut sequence = Sequence::new();
    sequence.append_element(send_message_element(1, Some(receiver), 77));
    sequence.append_element(SequenceElement::new_generic(
        2,
        Command::Unblip,
        Some(receiver),
    ));
    let sequence_id = engine.orders.sequence_manager.launch_sequence(sequence);

    let mut display = crate::engine::HostDisplayState::default();
    engine.drain_pending_immediate_actions_sync(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::new(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .expect("message element")
            .state,
        SequenceState::Terminated
    );
    let successor_state = engine
        .orders
        .sequence_manager
        .get_element(sequence_id, 1)
        .expect("successor element")
        .state;
    assert!(
        !engine
            .get_entity(receiver)
            .expect("receiver")
            .element_data()
            .blipped,
        "the successor must still execute after the required receiver VM is absent; state={successor_state:?}"
    );
}
