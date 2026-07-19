use super::*;

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
