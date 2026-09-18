use super::*;

#[test]
fn nested_sequence_actions_finish_before_parent_tail() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let mut ordering = scripted_soldier("OrderingReceiver");
    ordering.element_data_mut().blipped = true;
    let ordering_id = engine.add_test_entity(ordering);
    let ordering_handle = ScriptHandleCodec::actor_handle(ordering_id);
    engine
        .scripts
        .mission
        .as_mut()
        .expect("script installed")
        .bind_actor(ordering_handle, "OrderingReceiver");

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

    assert_eq!(
        engine.scripts.globals.get(904),
        Some(&0),
        "nested ProcessMessage ran before the parent's later Unblip"
    );
    assert_eq!(
        engine.scripts.globals.get(907),
        Some(&0),
        "nested LockAI sequence completed and resumed before parent Unblip"
    );
    let actor = engine.ent(ordering_id);
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
fn child_dispatch_failure_leaves_parent_successor_unexecuted() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let failure_id = engine.add_test_entity(scripted_soldier("FailureReceiver"));
    engine.elem_mut(failure_id).blipped = true;
    let failure_handle = bind_script_actor(&mut engine, failure_id, "FailureReceiver");
    let missing_id = engine.add_test_entity(scripted_soldier(""));
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
    assert!(parent_unblip.is_some(), "parent Unblip remains unexecuted");
    assert!(
        engine.elem(failure_id).blipped,
        "parent successor must not execute after the child error"
    );
}

#[test]
fn open_scroll_terminates_before_nested_child_failure() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    let mut scroll = crate::element::ElementScroll::default();
    scroll.element.kind = ElementKind::ObjectScroll;
    scroll.element.active = true;
    let scroll_id = engine.add_test_entity(Entity::Scroll(scroll));
    let scroll_handle = ScriptHandleCodec::actor_handle(scroll_id);
    let reader_id = engine.add_test_entity(scripted_soldier(""));
    engine.elem_mut(reader_id).blipped = true;

    engine
        .scripts
        .mission
        .as_mut()
        .expect("script installed")
        .bind_scroll(scroll_handle, "OpenScrollFailure");

    let mut open_scroll = SequenceElement::new_generic(1, Command::OpenScroll, None);
    open_scroll.set_property(Field::Scroll, FieldValue::Element(scroll_id));
    open_scroll.set_property(Field::ScrollReader, FieldValue::Element(reader_id));
    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(open_scroll);
    sequence.append_element(SequenceElement::new(1, Command::Unblip, Some(reader_id)));
    let sequence_id = engine.orders.sequence_manager.insert_sequence(sequence);

    let error = engine
        .start_sequence_inline(
            &crate::sim_rng::test_context(),
            &assets,
            &mut Vec::new(),
            sequence_id,
        )
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
        engine.elem(reader_id).blipped,
        "the parent successor was not executed after the error"
    );
}

#[test]
fn local_open_scroll_vm_failure_marks_it_impossible_without_starting_successor() {
    let (mut engine, reader_id, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    engine.elem_mut(reader_id).blipped = true;

    let mut scroll = crate::element::ElementScroll::default();
    scroll.element.kind = ElementKind::ObjectScroll;
    scroll.element.active = true;
    let scroll_id = engine.add_test_entity(Entity::Scroll(scroll));
    let scroll_handle = ScriptHandleCodec::actor_handle(scroll_id);
    engine
        .scripts
        .mission
        .as_mut()
        .expect("script installed")
        .bind_scroll(scroll_handle, "OpenScrollLocalFailure");

    let mut open_scroll = SequenceElement::new_generic(1, Command::OpenScroll, None);
    open_scroll.set_property(Field::Scroll, FieldValue::Element(scroll_id));
    open_scroll.set_property(Field::ScrollReader, FieldValue::Element(reader_id));
    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(open_scroll);
    sequence.append_element(SequenceElement::new(2, Command::Unblip, Some(reader_id)));
    let sequence_id = engine.orders.sequence_manager.insert_sequence(sequence);

    let error = engine
        .start_sequence_inline(
            &crate::sim_rng::test_context(),
            &assets,
            &mut Vec::new(),
            sequence_id,
        )
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
        engine.elem(reader_id).blipped,
        "the Unblip successor must not execute after local OpenScroll failure"
    );
}

#[test]
fn scroll_send_message_preserves_this_scroll_through_child_and_resume() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let observer_id = engine.add_test_entity(scripted_soldier("ScrollObserver"));
    let observer_handle = ScriptHandleCodec::actor_handle(observer_id);
    let scroll_handle = 0x1A2B_3C4D;
    let script = engine.scripts.mission.as_mut().expect("script installed");
    script.bind_actor(observer_handle, "ScrollObserver");
    script.bind_scroll(scroll_handle, "ScrollRelay");

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

    let globals = &engine.scripts.globals;
    assert_eq!(globals.get(905), Some(&scroll_handle));
    assert_eq!(globals.get(906), Some(&scroll_handle));
}

#[test]
fn scroll_ownerless_send_message_preserves_this_scroll_in_global_and_parent() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let scroll_handle = 0x1020_3040;
    engine
        .scripts
        .mission
        .as_mut()
        .expect("script installed")
        .bind_scroll(scroll_handle, "ScrollRelay");
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
    let globals = &engine.scripts.globals;
    assert_eq!(globals.get(902), Some(&66));
    assert_eq!(globals.get(908), Some(&scroll_handle));
    assert_eq!(globals.get(909), Some(&scroll_handle));
}
