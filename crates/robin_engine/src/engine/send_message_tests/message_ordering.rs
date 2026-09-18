use super::*;
use crate::engine::TickCtx;

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

    engine.ai_ctrl_mut(receiver).script_locked = true;

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
    let old_id = engine.t_launch_sequence_with(TickCtx::new(&sim, &assets), old_sequence);
    engine.hourglass_phase_sequences(TickCtx::new(&sim, &assets), &mut display);

    assert_eq!(
        engine.world.entities.current_element_for_actor(receiver),
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
    let replacement_id = engine.t_launch_sequence_with(TickCtx::new(&sim, &assets), replacement);
    engine.hourglass_phase_sequences(TickCtx::new(&sim, &assets), &mut display);

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
        engine.world.entities.current_element_for_actor(receiver),
        Some((replacement_id, 1)),
        "the new PlayAnim must become the actor's live command"
    );

    let ai = engine.ai_ctrl(receiver);
    assert!(ai.script_locked, "the replacement lock must remain held");
}

#[test]
fn script_send_message_sequence_does_not_preempt_current_actor_element() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let (mut engine, receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    let active_id = engine.launch_element(
        TickCtx::new(sim, &assets),
        SequenceElement::new_movement(1, Command::Move, Some(receiver), OrderType::RunningUpright),
    );
    engine.select_sequence_element(receiver, Some((active_id, 0)));
    engine.t_element_in_progress(&assets, active_id, 0);
    assert_eq!(
        engine.world.entities.current_element_for_actor(receiver),
        Some((active_id, 0))
    );

    let frame_before = engine.control.frame_counter;
    engine
        .call_external_native(
            TickCtx::new(sim, &assets),
            "SendMessageWithArguments",
            &[handle, 1234, 55, -7],
        )
        .expect("SendMessageWithArguments should complete synchronously");

    assert_eq!(
        engine.control.frame_counter, frame_before,
        "SendMessage is zero-frame"
    );
    assert_eq!(
        engine.world.entities.current_element_for_actor(receiver),
        Some((active_id, 0)),
        "immediate execution bypasses instruction contention and preserves the current element"
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

    // Original-game message sending launches a sequence element, whose
    // The send-message command's immediate path invokes message processing
    // inline.
    engine
        .call_external_native(TickCtx::new(sim, &assets), "SendMessage", &[handle, 314])
        .expect("SendMessage should complete synchronously");

    assert_eq!(
        engine.scripts.globals.get(900),
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
        .call_external_native(TickCtx::new(sim, &assets), "SendMessage", &[handle, 41])
        .expect("first SendMessage");
    engine
        .call_external_native(TickCtx::new(sim, &assets), "SendMessage", &[handle, 72])
        .expect("second SendMessage");

    assert_eq!(engine.control.frame_counter, frame_before);
    assert_eq!(
        engine.scripts.globals.get(900),
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
    let receiver = engine.add_test_entity(scripted_soldier("OrderingReceiver"));
    let handle = bind_script_actor(&mut engine, receiver, "OrderingReceiver");
    engine.elem_mut(receiver).blipped = true;

    let mut sequence = Sequence::new();
    sequence.append_element(send_message_element(1, Some(receiver), 77));
    sequence.append_element(SequenceElement::new_generic(
        1,
        Command::Unblip,
        Some(receiver),
    ));
    engine.t_launch_sequence(&LevelAssets::new(), sequence);

    assert_eq!(
        engine.scripts.globals.get(904),
        Some(&0),
        "ProcessMessage must observe state before the later Unblip sibling"
    );
    assert!(!engine.elem(receiver).blipped);
    assert_eq!(ScriptHandleCodec::actor_handle(receiver), handle);
}

#[test]
fn target_activation_callback_precedes_later_engine_sibling() {
    let (mut engine, reader, _) = engine_with_receiver();
    let target = engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::Target;
            initial_element.active = true;
            initial_element
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
    engine.t_launch_sequence(&LevelAssets::new(), sequence);

    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(
        TickCtx::new(&crate::sim_rng::test_context(), &LevelAssets::new()),
        &mut display,
    );

    assert!(
        !engine.actors_frozen(),
        "ActivatedByArrow's callback must run before the later unfreeze sibling"
    );
}

#[test]
fn send_message_callback_precedes_later_move_translation() {
    let (mut engine, _, _) = engine_with_receiver();
    let mover = engine.add_test_entity(scripted_soldier("MoveOrdering"));
    let mover_handle = bind_script_actor(&mut engine, mover, "MoveOrdering");
    engine.place_map(mover, crate::coordinates::MapPoint::new(0.0, 0.0));
    engine.ent_mut(mover).position_iface_mut().set_move_box(
        crate::coordinates::MoveBox::from_coords(-4.0, -4.0, 4.0, 4.0),
    );

    let mut movement =
        SequenceElement::new_movement(1, Command::Move, Some(mover), OrderType::WalkingUpright);
    if let crate::sequence::SequenceElementData::Movement { destination, .. } = &mut movement.data {
        *destination = crate::coordinates::MapPoint::new(20.0, 0.0);
    }
    let mut sequence = Sequence::new();
    sequence.append_element(send_message_element(1, Some(mover), 79));
    sequence.append_element(movement);
    let sequence_id = engine.t_launch_sequence(&LevelAssets::new(), sequence);

    let assets = engine.test_runtime_assets();
    engine.hourglass_phase_sequences(
        TickCtx::new(&crate::sim_rng::test_context(), &assets),
        &mut crate::engine::HostDisplayState::default(),
    );

    assert_eq!(
        engine.scripts.globals.get(909),
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
fn ownerless_message_runs_wait_successor_before_next_launch() {
    let (mut engine, receiver, _) = engine_with_receiver();
    engine.elem_mut(receiver).blipped = true;

    let mut message_then_wait = Sequence::new();
    message_then_wait.append_element(send_message_element(1, None, 80));
    let mut wait = SequenceElement::new(2, Command::Wait, Some(receiver));
    wait.priority = crate::sequence::SequencePriority::Wait;
    message_then_wait.append_element(wait);
    let sequence_id = engine.t_launch_sequence(&LevelAssets::new(), message_then_wait);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 1)
            .unwrap()
            .state,
        SequenceState::InProgress,
        "WAIT successor completes inside the first launch",
    );
    assert!(engine.elem(receiver).blipped);

    let mut older_sibling = Sequence::new();
    older_sibling.append_element(SequenceElement::new_generic(
        1,
        Command::Unblip,
        Some(receiver),
    ));
    engine.t_launch_sequence(&LevelAssets::new(), older_sibling);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 1)
            .expect("WAIT successor")
            .state,
        SequenceState::InProgress,
        "Ready must run the WAIT successor before returning"
    );
    assert!(
        !engine.elem(receiver).blipped,
        "the next immediate launch runs after the WAIT successor"
    );
}

#[test]
fn recorded_actor_message_closes_ready_before_parent_vm_resumes() {
    let (mut engine, _, _) = engine_with_receiver();
    let actor = engine.add_test_entity(scripted_soldier("OrderingReceiver"));
    let handle = bind_script_actor(&mut engine, actor, "OrderingReceiver");
    engine.elem_mut(actor).blipped = true;

    engine
        .call_script_vm(
            TickCtx::new(&crate::sim_rng::test_context(), &LevelAssets::new()),
            super::ScriptVmKey::Actor(handle),
            "TriggerNextLevel",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("recorded SendMessage successor should finish before Thanx resumes");

    assert_eq!(
        engine.scripts.globals.get(908),
        Some(&1),
        "the parent VM must observe the next-level Unblip successor"
    );
}

#[test]
fn missing_send_message_receiver_reports_failure_after_successor_cleanup() {
    let (mut engine, _, _) = engine_with_receiver();
    let receiver = engine.add_test_entity(scripted_soldier(""));
    engine.elem_mut(receiver).blipped = true;

    let mut sequence = Sequence::new();
    sequence.append_element(send_message_element(1, Some(receiver), 77));
    sequence.append_element(SequenceElement::new_generic(
        2,
        Command::Unblip,
        Some(receiver),
    ));
    let error = engine
        .launch_sequence_inline(
            TickCtx::new(&crate::sim_rng::test_context(), &LevelAssets::new()),
            &mut Vec::new(),
            sequence,
        )
        .expect_err("an unbound required receiver must report its error");
    assert!(format!("{error:?}").contains("required VM is not bound"));
    let sequence_id = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .find(|sequence| {
            sequence.elements.first().is_some_and(|element| {
                element.owner == Some(receiver) && element.command == Command::SendMessage
            })
        })
        .expect("failed message retains its sequence for cleanup inspection")
        .id;

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .expect("message element")
            .state,
        SequenceState::Impossible,
        "the registration boundary marks the failed required receiver explicitly"
    );
    let successor_state = engine
        .orders
        .sequence_manager
        .get_element(sequence_id, 1)
        .expect("successor element")
        .state;
    assert_eq!(
        successor_state,
        SequenceState::Impossible,
        "the reported failure cascades through the already completed successor"
    );
    assert!(
        !engine.elem(receiver).blipped,
        "the successor must still execute after the required receiver VM is absent; state={successor_state:?}"
    );
}
