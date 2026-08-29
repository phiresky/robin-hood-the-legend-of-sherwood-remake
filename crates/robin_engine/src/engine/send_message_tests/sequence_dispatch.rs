use super::*;

#[test]
fn nested_sequence_actions_finish_before_parent_tail() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let mut ordering = scripted_soldier("OrderingReceiver");
    ordering.element_data_mut().blipped = true;
    let ordering_id = engine.add_entity(ordering);
    let ordering_handle = ScriptHandleCodec::actor_handle(ordering_id);
    let simulation = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &simulation,
        &mut engine.world.entities,
        &mut engine.ai.global,
        std::sync::Arc::make_mut(&mut engine.world.fast_grid),
    );
    assert!(
        engine
            .scripts
            .mission
            .as_mut()
            .expect("script installed")
            .bind_actor(
                ordering_handle,
                "OrderingReceiver",
                &mut engine.script_domains,
                &capabilities,
            )
    );

    engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(ordering_handle),
            "TriggerParentOrder",
            &[],
            crate::natives::ScriptCallFrame::actor(ordering_handle),
        )
        .expect("nested sequence stack should drain depth-first");

    let script = engine.scripts.mission.as_ref().expect("script installed");
    assert_eq!(
        script.state.globals.get(&904),
        Some(&0),
        "nested ProcessMessage ran before the parent's later Unblip"
    );
    assert_eq!(
        script.state.globals.get(&907),
        Some(&0),
        "nested LockAI sequence completed and resumed before parent Unblip"
    );
    let actor = engine.get_entity(ordering_id).expect("ordering actor");
    assert!(!actor.element_data().blipped, "parent tail eventually ran");
    assert!(
        actor
            .ai_controller()
            .expect("ordering actor AI")
            .script_locked,
        "nested LockAI completed before control returned to the parent"
    );
}

#[test]
fn detached_parent_tail_is_restored_when_child_dispatch_fails() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let failure_id = engine.add_entity(scripted_soldier("FailureReceiver"));
    engine
        .get_entity_mut(failure_id)
        .unwrap()
        .element_data_mut()
        .blipped = true;
    let failure_handle = bind_script_actor(&mut engine, failure_id, "FailureReceiver");
    let missing_id = engine.add_entity(scripted_soldier(""));
    let missing_handle = ScriptHandleCodec::actor_handle(missing_id);
    let error = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(failure_handle),
            "TriggerFailure",
            &[missing_handle],
            crate::natives::ScriptCallFrame::actor(failure_handle),
        )
        .expect_err("nested child with a missing required VM must fail");
    assert!(
        error.contains("required VM is not bound"),
        "unexpected error: {error}"
    );

    let mut parent_send = None;
    let mut child_send = None;
    let mut parent_unblip = None;
    for sequence in engine.orders.sequence_manager.sequences_iter() {
        for (element_index, element) in sequence.elements.iter().enumerate() {
            match (element.command, element.state) {
                (Command::SendMessage, SequenceState::Terminated) => {
                    parent_send = Some((sequence.id, element_index));
                }
                (Command::SendMessage, SequenceState::Impossible) => {
                    child_send = Some((sequence.id, element_index));
                }
                (Command::Unblip, SequenceState::Todo) => {
                    parent_unblip = Some((sequence.id, element_index));
                }
                _ => {}
            }
        }
    }
    assert!(parent_send.is_some(), "successful ancestor is Terminated");
    assert!(child_send.is_some(), "only the actual child is Impossible");
    let parent_unblip = parent_unblip.expect("detached parent Unblip tail");
    assert!(
        engine
            .get_entity(failure_id)
            .unwrap()
            .element_data()
            .blipped,
        "parent tail was restored but not overtaken after the child error"
    );
    assert!(
        matches!(
            engine
                .orders
                .sequence_manager
                .pop_pending_immediate_action(),
            Some(SequenceAction::ExecuteImmediateOwner {
                sequence_id,
                element_index,
                ..
            }) if (sequence_id, element_index) == parent_unblip
        ),
        "the real native path restores the detached parent tail on error"
    );
}

#[test]
fn open_scroll_terminates_before_nested_child_failure_and_restores_tail() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    let mut scroll = crate::element::ElementScroll::default();
    scroll.element.kind = ElementKind::ObjectScroll;
    scroll.element.active = true;
    let scroll_id = engine.add_entity(Entity::Scroll(scroll));
    let scroll_handle = ScriptHandleCodec::actor_handle(scroll_id);
    let reader_id = engine.add_entity(scripted_soldier(""));
    engine
        .get_entity_mut(reader_id)
        .expect("reader")
        .element_data_mut()
        .blipped = true;

    let simulation = crate::sim_rng::test_context();

    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &simulation,
        &mut engine.world.entities,
        &mut engine.ai.global,
        std::sync::Arc::make_mut(&mut engine.world.fast_grid),
    );
    assert!(
        engine
            .scripts
            .mission
            .as_mut()
            .expect("script installed")
            .bind_scroll(
                scroll_handle,
                "OpenScrollFailure",
                &mut engine.script_domains,
                &capabilities,
            )
    );

    let mut open_scroll = SequenceElement::new_generic(1, Command::OpenScroll, None);
    open_scroll.set_property(Field::Scroll, FieldValue::Element(scroll_id));
    open_scroll.set_property(Field::ScrollReader, FieldValue::Element(reader_id));
    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(open_scroll);
    sequence.append_element(SequenceElement::new(1, Command::Unblip, Some(reader_id)));
    let sequence_id = engine.orders.sequence_manager.launch_sequence(sequence);

    let error = engine
        .drain_script_synchronous_actions(&crate::sim_rng::test_context(), &assets, &mut Vec::new())
        .expect_err("nested IsTaken SendMessage must fail on the missing reader VM");
    assert!(
        error.detail.contains("required VM is not bound"),
        "unexpected error: {}",
        error.detail
    );

    let sequence = engine
        .orders
        .sequence_manager
        .get_sequence(sequence_id)
        .expect("OpenScroll sequence");
    assert_eq!(sequence.elements[0].state, SequenceState::Terminated);
    assert_eq!(sequence.elements[1].state, SequenceState::Todo);
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .any(|sequence| {
                sequence.elements.iter().any(|element| {
                    element.command == Command::SendMessage
                        && element.state == SequenceState::Impossible
                })
            }),
        "only the nested SendMessage child is Impossible"
    );
    assert!(
        matches!(
            engine
                .orders
                .sequence_manager
                .pop_pending_immediate_action(),
            Some(SequenceAction::ExecuteImmediateOwner {
                sequence_id: pending_sequence,
                element_index: 1,
                ..
            }) if pending_sequence == sequence_id
        ),
        "the parent Unblip tail is restored after the child error"
    );
    assert!(
        engine
            .get_entity(reader_id)
            .expect("reader")
            .element_data()
            .blipped,
        "the restored parent tail was not executed after the error"
    );
}

#[test]
fn local_open_scroll_vm_failure_marks_it_impossible_without_starting_successor() {
    let (mut engine, reader_id, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    engine
        .get_entity_mut(reader_id)
        .expect("reader")
        .element_data_mut()
        .blipped = true;

    let mut scroll = crate::element::ElementScroll::default();
    scroll.element.kind = ElementKind::ObjectScroll;
    scroll.element.active = true;
    let scroll_id = engine.add_entity(Entity::Scroll(scroll));
    let scroll_handle = ScriptHandleCodec::actor_handle(scroll_id);
    let simulation = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &simulation,
        &mut engine.world.entities,
        &mut engine.ai.global,
        std::sync::Arc::make_mut(&mut engine.world.fast_grid),
    );
    assert!(
        engine
            .scripts
            .mission
            .as_mut()
            .expect("script installed")
            .bind_scroll(
                scroll_handle,
                "OpenScrollLocalFailure",
                &mut engine.script_domains,
                &capabilities,
            )
    );

    let mut open_scroll = SequenceElement::new_generic(1, Command::OpenScroll, None);
    open_scroll.set_property(Field::Scroll, FieldValue::Element(scroll_id));
    open_scroll.set_property(Field::ScrollReader, FieldValue::Element(reader_id));
    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(open_scroll);
    sequence.append_element(SequenceElement::new(2, Command::Unblip, Some(reader_id)));
    let sequence_id = engine.orders.sequence_manager.launch_sequence(sequence);

    let error = engine
        .drain_script_synchronous_actions(&crate::sim_rng::test_context(), &assets, &mut Vec::new())
        .expect_err("the malformed IsTaken VM must fail locally");
    assert!(
        error.detail.contains("stopped abnormally: RanOff"),
        "unexpected error: {}",
        error.detail
    );

    let sequence = engine
        .orders
        .sequence_manager
        .get_sequence(sequence_id)
        .expect("OpenScroll sequence");
    assert_eq!(sequence.elements[0].state, SequenceState::Impossible);
    assert_eq!(
        sequence.elements[1].state,
        SequenceState::Impossible,
        "sequence failure cancels the unstarted successor without dispatching it"
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .has_pending_immediate_actions(),
        "a locally failed OpenScroll must not register its level-2 successor"
    );
    assert!(
        engine
            .get_entity(reader_id)
            .expect("reader")
            .element_data()
            .blipped,
        "the Unblip successor must not execute after local OpenScroll failure"
    );
}

#[test]
fn scroll_send_message_preserves_this_scroll_through_child_and_resume() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let observer_id = engine.add_entity(scripted_soldier("ScrollObserver"));
    let observer_handle = ScriptHandleCodec::actor_handle(observer_id);
    let scroll_handle = 0x1A2B_3C4D;
    let simulation = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &simulation,
        &mut engine.world.entities,
        &mut engine.ai.global,
        std::sync::Arc::make_mut(&mut engine.world.fast_grid),
    );
    let script = engine.scripts.mission.as_mut().expect("script installed");
    assert!(script.bind_actor(
        observer_handle,
        "ScrollObserver",
        &mut engine.script_domains,
        &capabilities,
    ));
    assert!(script.bind_scroll(
        scroll_handle,
        "ScrollRelay",
        &mut engine.script_domains,
        &capabilities,
    ));

    let frame = crate::natives::ScriptCallFrame::default()
        .with_script_this(scroll_handle)
        .with_current_scroll(scroll_handle);
    engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Scroll(scroll_handle),
            "TriggerScroll",
            &[observer_handle],
            frame,
        )
        .expect("scroll→actor message should preserve the caller frame");

    let globals = &engine
        .scripts
        .mission
        .as_ref()
        .expect("script installed")
        .state
        .globals;
    assert_eq!(globals.get(&905), Some(&scroll_handle));
    assert_eq!(globals.get(&906), Some(&scroll_handle));
}

#[test]
fn scroll_ownerless_send_message_preserves_this_scroll_in_global_and_parent() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let scroll_handle = 0x1020_3040;
    let simulation = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &simulation,
        &mut engine.world.entities,
        &mut engine.ai.global,
        std::sync::Arc::make_mut(&mut engine.world.fast_grid),
    );
    engine
        .scripts
        .mission
        .as_mut()
        .expect("script installed")
        .bind_scroll(
            scroll_handle,
            "ScrollRelay",
            &mut engine.script_domains,
            &capabilities,
        );
    let frame = crate::natives::ScriptCallFrame::default()
        .with_script_this(scroll_handle)
        .with_current_scroll(scroll_handle);
    engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Scroll(scroll_handle),
            "TriggerOwnerless",
            &[],
            frame,
        )
        .expect("scroll→global message should preserve caller frame");
    let globals = &engine.scripts.mission.as_ref().unwrap().state.globals;
    assert_eq!(globals.get(&902), Some(&66));
    assert_eq!(globals.get(&908), Some(&scroll_handle));
    assert_eq!(globals.get(&909), Some(&scroll_handle));
}
