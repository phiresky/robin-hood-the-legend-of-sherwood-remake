use super::*;

fn make_simple_element(level: u16, cmd: Command, owner: Option<EntityId>) -> SequenceElement {
    SequenceElement::new(level, cmd, owner)
}

/// SequenceManager unit tests have no EngineInner owner callback. Resume the
/// synchronous SetState continuation at the point where that callback would
/// have returned.
fn finish_test_condolations(mgr: &mut SequenceManager) {
    loop {
        let pending = mgr.drain_pending_condolations();
        if pending.is_empty() {
            break;
        }
        for dispatch in pending {
            mgr.finish_pending_condolation(dispatch);
        }
    }
}

#[test]
fn replacement_interruption_marks_outgoing_card_unselected() {
    let owner = EntityId::Pc(crate::entity_id::PcId(7));
    let mut manager = SequenceManager::new();
    let outgoing = manager.launch_element(SequenceElement::new(1, Command::Move, Some(owner)));
    manager.element_in_progress(outgoing, 0);

    manager.element_interrupted_after_replacement_selected(outgoing, 0, CascadeFlags::NEXT_LEVEL);

    let pending = manager.drain_pending_condolations();
    assert_eq!(pending.len(), 1);
    assert!(!pending[0].card.was_selected);
}

#[test]
fn sequence_command_level_grouping() {
    let mut seq = Sequence::new();

    // Level 1: two elements (run in parallel)
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    seq.append_element(make_simple_element(
        1,
        Command::WaitTimer,
        Some(EntityId::Pc(crate::entity_id::PcId(1))),
    ));

    // Level 2: one element (waits for level 1)
    seq.append_element(make_simple_element(
        2,
        Command::PassDoor,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));

    assert_eq!(seq.len(), 3);
    assert!(!seq.is_empty());
}

#[test]
fn sequence_launch_and_advance() {
    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    seq.append_element(make_simple_element(
        1,
        Command::WaitTimer,
        Some(EntityId::Pc(crate::entity_id::PcId(1))),
    ));
    seq.append_element(make_simple_element(
        2,
        Command::PassDoor,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));

    assert!(seq.launch());

    // First call should return both level-1 elements
    let to_go = seq.next_elements_go();
    assert_eq!(to_go.len(), 2);
    assert_eq!(to_go[0], 0); // element index 0
    assert_eq!(to_go[1], 1); // element index 1
    assert_eq!(seq.running_elements, 2);

    // Simulate first element finishing
    let advance = seq.element_ready();
    assert!(!advance); // still one running

    // Second element finishes
    let advance = seq.element_ready();
    assert!(advance); // all done at this level

    // Next level starts
    let to_go = seq.next_elements_go();
    assert_eq!(to_go.len(), 1);
    assert_eq!(to_go[0], 2); // element index 2
}

/// The `RecordPlayAnim*` natives at natives/mod.rs:2680-2729 write
/// `Field::AnimationId` as `FieldValue::Animation(OrderType)` on a
/// generic sequence element.  The `Command::PlayAnim*` dispatch in
/// `tick.rs` reads it back out via `get_property` and destructures
/// the same variant to feed `force_animation`.  Verify the
/// round-trip end-to-end.
#[test]
fn animation_id_property_roundtrip() {
    use crate::order::OrderType;

    let cases = [
        (Command::PlayAnim, OrderType::WaitingUpright),
        (Command::PlayAnimLoop, OrderType::WaitingCrouched),
        (Command::PlayAnimFreeze, OrderType::Taking),
        (Command::PlayAnimFrozen, OrderType::Pointing),
    ];
    for (cmd, anim) in cases {
        let mut elem = SequenceElement::new_generic(1, cmd, None);
        elem.set_property(Field::AnimationId, FieldValue::Animation(anim));
        let got = elem
            .get_property(Field::AnimationId)
            .expect("AnimationId round-trips via get_property");
        match got {
            FieldValue::Animation(a) => assert_eq!(*a, anim, "cmd {cmd:?}"),
            other => panic!("expected FieldValue::Animation, got {other:?}"),
        }
    }
}

#[test]
fn sequence_is_to_be_deleted() {
    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));

    // Todo element → not deletable
    assert!(!seq.is_to_be_deleted());

    // Mark as terminated → deletable
    seq.elements[0].state = SequenceState::Terminated;
    assert!(seq.is_to_be_deleted());
}

#[test]
fn sequence_has_owner() {
    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(5))),
    ));
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(3))),
    ));

    assert!(seq.has_owner(EntityId::Pc(crate::entity_id::PcId(5))));
    assert!(seq.has_owner(EntityId::Pc(crate::entity_id::PcId(3))));
    assert!(!seq.has_owner(EntityId::Pc(crate::entity_id::PcId(99))));

    // Terminated elements don't count
    seq.elements[0].state = SequenceState::Terminated;
    assert!(!seq.has_owner(EntityId::Pc(crate::entity_id::PcId(5))));
}

#[test]
fn state_change_inprogress() {
    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));

    let effects = seq.set_element_state(0, SequenceState::InProgress, CascadeFlags::NEXT_LEVEL);
    assert!(effects.increment_in_progress);
    assert!(!effects.decrement_in_progress);
    assert!(!effects.signal_ready);
}

#[test]
fn state_change_terminated_signals_ready() {
    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    // Must first go to InProgress
    seq.set_element_state(0, SequenceState::InProgress, CascadeFlags::NEXT_LEVEL);

    let effects = seq.set_element_state(0, SequenceState::Terminated, CascadeFlags::NEXT_LEVEL);
    assert!(effects.signal_ready);
    assert!(effects.decrement_in_progress);
    assert_eq!(
        effects.notify_owner,
        Some(EntityId::Pc(crate::entity_id::PcId(0)))
    );
}

#[test]
fn state_change_interrupted_cascades() {
    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    seq.append_element(make_simple_element(
        2,
        Command::PassDoor,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));

    let effects = seq.set_element_state(0, SequenceState::Interrupted, CascadeFlags::NEXT_LEVEL);

    // Should cascade to the next level (element 1)
    assert_eq!(effects.cascade.len(), 1);
    assert_eq!(effects.cascade[0].0, 1); // element index 1
}

#[test]
fn state_change_interrupted_does_not_resume_postponed_elements() {
    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    seq.append_element(make_simple_element(
        1,
        Command::Wait,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    seq.elements[0].postponed_element_index = Some(1);
    let cross = (SequenceId(91), 3);
    seq.elements[0].cross_postponed = Some(cross);

    let effects = seq.set_element_state(0, SequenceState::Interrupted, CascadeFlags::empty());

    assert_eq!(effects.start_postponed, None);
    assert_eq!(effects.resume_cross_postponed, None);
    assert_eq!(seq.elements[0].postponed_element_index, Some(1));
    assert_eq!(seq.elements[0].cross_postponed, Some(cross));
    assert_eq!(seq.elements[1].state, SequenceState::Todo);
}

#[test]
fn manager_launch_and_hourglass() {
    let mut mgr = SequenceManager::new();

    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    seq.append_element(make_simple_element(
        2,
        Command::Turn,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));

    let seq_id = mgr.launch_sequence(seq);

    // hourglass should return an action for the first element
    let actions = mgr.hourglass();
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        SequenceAction::InstructOwner {
            owner,
            sequence_id,
            element_index,
        } => {
            assert_eq!(*owner, EntityId::Pc(crate::entity_id::PcId(0)));
            assert_eq!(*sequence_id, seq_id);
            assert_eq!(*element_index, 0);
        }
        other => panic!("expected InstructOwner, got {:?}", other),
    }

    // No more pending
    let actions = mgr.hourglass();
    assert!(actions.is_empty());
}

/// Original `RHSequence::NextSequenceElementsGo`
/// (`original-code/RHsequence.cpp:235-289`) advances the cursor and
/// running count first, then walks that stable range in element order:
/// WAIT calls `Go()` inline, NORMAL is registered on the manager FIFO,
/// and an immediate command executes inside that registration. A WAIT
/// callback may terminate synchronously, enter `Ready()`, and dispatch a
/// WAIT successor before the outer launch/callback chain unwinds.
#[test]
fn wait_go_is_emitted_at_launch_return_in_registration_order_and_reentrant() {
    let mut mgr = SequenceManager::new();
    let wait_owner = EntityId::Pc(crate::entity_id::PcId(1));

    let mut sequence = Sequence::new();
    let mut normal = make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    );
    normal.priority = SequencePriority::Normal;
    sequence.append_element(normal);

    let mut wait = make_simple_element(1, Command::Wait, Some(wait_owner));
    wait.priority = SequencePriority::Wait;
    sequence.append_element(wait);

    let mut immediate = make_simple_element(1, Command::LockUser, None);
    immediate.priority = SequencePriority::Normal;
    sequence.append_element(immediate);

    let sequence_id = mgr.launch_sequence(sequence);

    let launched = mgr
        .get_sequence(sequence_id)
        .expect("sequence is registered");
    assert!(launched.started);
    assert_eq!(launched.cursor, 3);
    assert_eq!(launched.running_elements, 3);
    assert!(
        launched
            .elements
            .iter()
            .all(|element| element.state == SequenceState::Todo),
        "emitting Go must not invent the callback's state transition"
    );
    assert_eq!(
        mgr.elements_to_go.iter().copied().collect::<Vec<_>>(),
        vec![(sequence_id, 0)],
        "only NORMAL non-immediate work belongs on the hourglass FIFO"
    );

    // The loop stops at the WAIT `Go()`: element 2's registration — and
    // therefore its `ExecutedImmediately()` execution — is a later
    // iteration of the same C++ loop and only runs once that `Go()` has
    // returned.
    let synchronous = mgr.take_settled_synchronous_actions();
    assert_eq!(synchronous.len(), 1);
    assert!(matches!(
        synchronous[0],
        SequenceAction::InstructOwner {
            owner,
            sequence_id: action_sequence_id,
            element_index: 1,
        } if owner == wait_owner && action_sequence_id == sequence_id
    ));
    let after_wait_go = mgr.take_settled_synchronous_actions();
    assert_eq!(after_wait_go.len(), 1);
    assert!(matches!(
        after_wait_go[0],
        SequenceAction::ExecuteImmediateEngine {
            sequence_id: action_sequence_id,
            element_index: 2,
        } if action_sequence_id == sequence_id
    ));
    let deferred = mgr.hourglass();
    assert_eq!(deferred.len(), 1);
    assert!(matches!(
        deferred[0],
        SequenceAction::InstructOwner {
            sequence_id: action_sequence_id,
            element_index: 0,
            ..
        } if action_sequence_id == sequence_id
    ));

    // Re-entrant completion: the callback for level 1 reaches Ready(),
    // which must emit the level-2 WAIT into the synchronous stream. It
    // must not fall back onto `elements_to_go` for another hourglass.
    let mut mgr = SequenceManager::new();
    let mut sequence = Sequence::new();
    let mut first = make_simple_element(1, Command::Wait, None);
    first.priority = SequencePriority::Wait;
    sequence.append_element(first);
    let mut successor = make_simple_element(2, Command::Wait, None);
    successor.priority = SequencePriority::Wait;
    sequence.append_element(successor);

    let sequence_id = mgr.launch_sequence(sequence);
    let first_action = mgr.take_settled_synchronous_actions();
    assert!(matches!(
        first_action.as_slice(),
        [SequenceAction::EngineCommand {
            sequence_id: action_sequence_id,
            element_index: 0,
        }] if *action_sequence_id == sequence_id
    ));

    mgr.element_in_progress(sequence_id, 0);
    mgr.element_terminated(sequence_id, 0);

    let successor_action = mgr.take_settled_synchronous_actions();
    assert!(matches!(
        successor_action.as_slice(),
        [SequenceAction::EngineCommand {
            sequence_id: action_sequence_id,
            element_index: 1,
        }] if *action_sequence_id == sequence_id
    ));
    assert!(mgr.elements_to_go.is_empty());
    assert!(mgr.hourglass().is_empty());
    assert_eq!(
        mgr.get_element(sequence_id, 0)
            .expect("first element")
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        mgr.get_element(sequence_id, 1)
            .expect("successor element")
            .state,
        SequenceState::Todo
    );
}

#[test]
fn manager_wait_priority_go_bypasses_executed_immediately() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Pc(crate::entity_id::PcId(0));
    let mut wait = make_simple_element(1, Command::Speak, Some(owner));
    wait.priority = SequencePriority::Wait;

    let seq_id = mgr.launch_element(wait);

    let actions = mgr.take_settled_synchronous_actions();
    assert_eq!(actions.len(), 1);
    assert!(matches!(
        actions[0],
        SequenceAction::InstructOwner {
            owner: action_owner,
            sequence_id,
            element_index: 0,
        } if action_owner == owner && sequence_id == seq_id
    ));
    assert!(
        mgr.hourglass().is_empty(),
        "WAIT-priority Go must not remain deferred on elements_to_go"
    );
}

#[test]
fn manager_reentrant_wait_completion_queues_next_level_inline() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Pc(crate::entity_id::PcId(0));
    let mut first = make_simple_element(1, Command::Wait, Some(owner));
    first.priority = SequencePriority::Wait;
    let mut second = make_simple_element(2, Command::Wait, Some(owner));
    second.priority = SequencePriority::Wait;
    let mut seq = Sequence::new();
    seq.append_element(first);
    seq.append_element(second);

    let seq_id = mgr.launch_sequence(seq);
    let first_actions = mgr.take_settled_synchronous_actions();
    assert!(matches!(
        first_actions.as_slice(),
        [SequenceAction::InstructOwner {
            sequence_id,
            element_index: 0,
            ..
        }] if *sequence_id == seq_id
    ));

    // Simulate an Instruct callback that starts and completes the first
    // WAIT before the outer synchronous action drain returns. The owner
    // condolation is itself a synchronous boundary; finishing it resumes
    // Ready() and opens level 2 re-entrantly.
    mgr.element_in_progress(seq_id, 0);
    mgr.element_terminated(seq_id, 0);
    finish_test_condolations(&mut mgr);

    let reentrant_actions = mgr.take_settled_synchronous_actions();
    assert!(matches!(
        reentrant_actions.as_slice(),
        [SequenceAction::InstructOwner {
            owner: action_owner,
            sequence_id,
            element_index: 1,
        }] if *action_owner == owner && *sequence_id == seq_id
    ));
    assert!(
        mgr.hourglass().is_empty(),
        "the re-entrant next-level WAIT must not wait for hourglass"
    );
}

#[test]
fn manager_element_terminated_advances() {
    let mut mgr = SequenceManager::new();

    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    seq.append_element(make_simple_element(
        2,
        Command::Turn,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));

    let seq_id = mgr.launch_sequence(seq);

    // Drain the first hourglass
    let _ = mgr.hourglass();

    // Mark element 0 as in-progress then terminated
    mgr.element_in_progress(seq_id, 0);
    mgr.element_terminated(seq_id, 0);
    finish_test_condolations(&mut mgr);

    // The next level's element should now be queued
    let actions = mgr.hourglass();
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        SequenceAction::InstructOwner { element_index, .. } => assert_eq!(*element_index, 1),
        other => panic!("expected InstructOwner for element 1, got {:?}", other),
    }
}

#[test]
fn live_hourglass_places_normal_successor_after_older_fifo_work() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Pc(crate::entity_id::PcId(319));

    let mut route = Sequence::new();
    route.append_element(make_simple_element(1, Command::AssertPosition, Some(owner)));
    route.append_element(make_simple_element(2, Command::Move, Some(owner)));
    let route_id = mgr.launch_sequence(route);

    let older_owner_a = EntityId::Soldier(crate::entity_id::SoldierId(4));
    let older_owner_b = EntityId::Soldier(crate::entity_id::SoldierId(5));
    let older_a = mgr.launch_element(make_simple_element(
        1,
        Command::LookLeft,
        Some(older_owner_a),
    ));
    let older_b = mgr.launch_element(make_simple_element(
        1,
        Command::LookRight,
        Some(older_owner_b),
    ));

    assert!(matches!(
        mgr.pop_next_hourglass_action(),
        Some(SequenceAction::InstructOwner {
            sequence_id,
            element_index: 0,
            ..
        }) if sequence_id == route_id
    ));

    // AssertPosition terminates inside Actor::Translate. Ready registers
    // the level-2 Move at the live manager FIFO tail before Go returns.
    mgr.element_in_progress(route_id, 0);
    mgr.element_terminated(route_id, 0);
    finish_test_condolations(&mut mgr);

    let remaining = [
        mgr.pop_next_hourglass_action(),
        mgr.pop_next_hourglass_action(),
        mgr.pop_next_hourglass_action(),
    ];
    assert!(matches!(
        remaining[0],
        Some(SequenceAction::InstructOwner { sequence_id, .. }) if sequence_id == older_a
    ));
    assert!(matches!(
        remaining[1],
        Some(SequenceAction::InstructOwner { sequence_id, .. }) if sequence_id == older_b
    ));
    assert!(matches!(
        remaining[2],
        Some(SequenceAction::InstructOwner {
            owner: action_owner,
            sequence_id,
            element_index: 1,
        }) if action_owner == owner && sequence_id == route_id
    ));
}

#[test]
fn released_cross_postponed_action_keeps_owner_fifo_behind_ready_successor() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Pc(crate::entity_id::PcId(0));

    let mut old = Sequence::new();
    old.append_element(make_simple_element(1, Command::PassDoor, Some(owner)));
    old.append_element(make_simple_element(2, Command::Move, Some(owner)));
    let old_id = mgr.launch_sequence(old);
    let _ = mgr.hourglass();
    mgr.element_in_progress(old_id, 0);

    let replacement_id =
        mgr.launch_element(make_simple_element(1, Command::AssertPosition, Some(owner)));
    let _ = mgr.hourglass();
    mgr.postpone_element(replacement_id, 0);
    mgr.get_element_mut(old_id, 0).unwrap().cross_postponed = Some((replacement_id, 0));

    mgr.element_terminated(old_id, 0);
    finish_test_condolations(&mut mgr);

    let actions = mgr
        .take_deferred_owner_actions_through(owner, replacement_id, 0)
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [
            SequenceAction::InstructOwner {
                sequence_id: first_sequence,
                element_index: 1,
                ..
            },
            SequenceAction::InstructOwner {
                sequence_id: second_sequence,
                element_index: 0,
                ..
            }
        ] if *first_sequence == old_id && *second_sequence == replacement_id
    ));
}

#[test]
fn released_same_sequence_postponed_action_clears_blocker_edge() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Soldier(crate::entity_id::SoldierId(0));

    let mut sequence = Sequence::new();
    sequence.append_element(make_simple_element(1, Command::Wait, Some(owner)));
    sequence.append_element(make_simple_element(1, Command::AssertPosition, Some(owner)));
    let sequence_id = mgr.launch_sequence(sequence);
    let _ = mgr.hourglass();

    mgr.element_in_progress(sequence_id, 0);
    mgr.postpone_element(sequence_id, 1);
    mgr.get_element_mut(sequence_id, 0)
        .unwrap()
        .postponed_element_index = Some(1);

    mgr.element_terminated(sequence_id, 0);
    finish_test_condolations(&mut mgr);

    assert_eq!(
        mgr.get_element(sequence_id, 0)
            .unwrap()
            .postponed_element_index,
        None,
        "StartPostponedSequenceElement must detach the released edge"
    );
    assert!(matches!(
        mgr.hourglass().as_slice(),
        [SequenceAction::InstructOwner {
            owner: action_owner,
            sequence_id: action_sequence,
            element_index: 1,
        }] if *action_owner == owner && *action_sequence == sequence_id
    ));
}

#[test]
fn finishing_condolation_stops_at_nested_card_before_cascade_continues() {
    let mut mgr = SequenceManager::new();
    let owner = Some(EntityId::Pc(crate::entity_id::PcId(0)));
    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(1, Command::Move, owner));
    seq.append_element(make_simple_element(2, Command::Turn, owner));
    seq.append_element(make_simple_element(3, Command::LookLeft, owner));
    let seq_id = mgr.launch_sequence(seq);

    mgr.element_interrupted(seq_id, 0, CascadeFlags::NEXT_LEVEL);
    let mut pending = mgr.drain_pending_condolations();
    assert_eq!(pending.len(), 1);

    // In RHSequenceElement::SetState, the outer card returns and the
    // cascade enters element 1, whose own SendCondolationCard must run
    // before CASCADE_FOLLOWING is allowed to touch element 2.
    mgr.finish_pending_condolation(pending.remove(0));

    assert_eq!(
        mgr.get_element(seq_id, 1).unwrap().state,
        SequenceState::Interrupted
    );
    assert_eq!(
        mgr.get_element(seq_id, 2).unwrap().state,
        SequenceState::Todo,
        "the nested owner callback is a synchronous boundary in the cascade"
    );
    let nested = mgr.drain_pending_condolations();
    assert_eq!(nested.len(), 1);
    assert_eq!(nested[0].card.elem_idx, 1);
}

#[test]
fn stop_owner_interrupts_actor_work_postponed_by_injury() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Soldier(crate::entity_id::SoldierId(7));

    // A preference action is current until an injury postpones it.
    let mut parry = make_simple_element(1, Command::ParrySword, Some(owner));
    parry.priority = SequencePriority::Preference;
    let parry_seq = mgr.launch_element(parry);
    let _ = mgr.hourglass();
    mgr.element_in_progress(parry_seq, 0);
    mgr.postpone_element(parry_seq, 0);

    // During the injury's terminal condolence callback, Actor::Stop sees
    // the injury as current in the original and recursively stops the
    // postponed parry. Model the cross-sequence postponed link explicitly.
    let mut injury = make_simple_element(1, Command::ReceiveSwordDamage, Some(owner));
    injury.priority = SequencePriority::Injury;
    let injury_seq = mgr.launch_element(injury);
    let _ = mgr.hourglass();
    mgr.element_in_progress(injury_seq, 0);
    mgr.get_element_mut(injury_seq, 0).unwrap().cross_postponed = Some((parry_seq, 0));

    mgr.stop_owner(owner, SequencePriority::Preference, &|elem| elem.priority);

    assert_eq!(
        mgr.get_element(injury_seq, 0).unwrap().state,
        SequenceState::InProgress,
        "Preference StopAll must not interrupt the stronger injury callback"
    );
    assert_eq!(
        mgr.get_element(parry_seq, 0).unwrap().state,
        SequenceState::Interrupted,
        "the parry hidden underneath the injury must not resume after StopAll"
    );
    assert_eq!(
        mgr.get_element(injury_seq, 0).unwrap().cross_postponed,
        None,
        "the injury must not retain a resumable link to stopped actor work"
    );
}

#[test]
fn split_stop_scans_work_registered_by_selected_element_callback() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Soldier(crate::entity_id::SoldierId(7));
    let resolver = |element: &SequenceElement| element.priority;

    let mut current = make_simple_element(1, Command::Turn, Some(owner));
    current.priority = SequencePriority::Normal;
    let current_seq = mgr.launch_element(current);
    let _ = mgr.hourglass();
    mgr.element_in_progress(current_seq, 0);

    mgr.stop_owner_current_from_root(
        owner,
        Some((current_seq, 0)),
        SequencePriority::Preference,
        &resolver,
    );
    assert_eq!(
        mgr.get_element(current_seq, 0).unwrap().state,
        SequenceState::Interrupted
    );

    // Model SendCondolationCard -> Think registering overview work while
    // Actor::Stop is still between its selected and pending phases.
    let mut callback_look = make_simple_element(1, Command::LookLeft, Some(owner));
    callback_look.priority = SequencePriority::Normal;
    let callback_look_seq = mgr.launch_element(callback_look);

    let pending_snapshot = mgr.pending_elements_for_owner(owner);

    // Model a card from the pending scan registering another command
    // after the scan captured its stable membership.
    let mut pending_card_look = make_simple_element(1, Command::LookRight, Some(owner));
    pending_card_look.priority = SequencePriority::Normal;
    let pending_card_look_seq = mgr.launch_element(pending_card_look);
    for root in pending_snapshot {
        mgr.stop_pending_element_from_root(owner, root, SequencePriority::Preference, &resolver);
    }
    mgr.compact_terminal_elements_to_go();
    assert_eq!(
        mgr.get_element(callback_look_seq, 0).unwrap().state,
        SequenceState::Interrupted,
        "StopNotYetLaunched must see work registered by the selected element's synchronous callback"
    );

    // Original snapshots the list size when StopNotYetLaunched begins.
    // Work registered by a captured entry's card belongs to the next
    // manager Hourglass, not this scan.
    assert_eq!(
        mgr.get_element(pending_card_look_seq, 0).unwrap().state,
        SequenceState::Todo
    );
    assert!(
        mgr.v48_elements_to_go()
            .contains(&(pending_card_look_seq, 0))
    );
}

#[test]
fn stop_owner_batches_cleanup_for_long_cross_postponed_chain() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Soldier(crate::entity_id::SoldierId(7));
    let unrelated_owner = EntityId::Soldier(crate::entity_id::SoldierId(8));

    // Replays retain a large amount of historical sequence state until the
    // Friday cleanup pass. Keep enough unrelated sequences here to catch a
    // regression that scans the whole manager for every stopped chain node.
    for _ in 0..4096 {
        mgr.launch_element(make_simple_element(1, Command::Wait, Some(unrelated_owner)));
    }

    let mut chain = Vec::with_capacity(4096);
    for _ in 0..4096 {
        let mut element = make_simple_element(1, Command::EnterSwordfight, Some(owner));
        element.priority = SequencePriority::Normal;
        let sequence = mgr.launch_element(element);
        mgr.postpone_element(sequence, 0);
        if let Some(&previous) = chain.last() {
            mgr.get_element_mut(previous, 0).unwrap().cross_postponed = Some((sequence, 0));
        }
        chain.push(sequence);
    }

    mgr.stop_owner_current_from_root(
        owner,
        Some((chain[0], 0)),
        SequencePriority::Preference,
        &|element| element.priority,
    );

    for sequence in chain {
        let element = mgr.get_element(sequence, 0).unwrap();
        assert_eq!(element.state, SequenceState::Interrupted);
        assert_eq!(element.cross_postponed, None);
    }
}

#[test]
fn repeated_preference_stops_skip_growing_strong_postponed_prefix() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Soldier(crate::entity_id::SoldierId(7));

    let mut root_element = make_simple_element(1, Command::QuitSwordfight, Some(owner));
    root_element.priority = SequencePriority::PostponeEverythingButInjuries;
    let root = mgr.launch_element(root_element);
    mgr.element_in_progress(root, 0);

    // EnterSwordfight can add one equal-priority postponed element between
    // successive PrepareToEnterSwordFight Stop(PREFERENCE) calls. Original's
    // pointer walk is effect-free for this graph; rescanning the full prefix
    // after every append makes the Rust representation triangular.
    let mut tail = root;
    let mut chain = Vec::with_capacity(8192);
    for _ in 0..8192 {
        let mut element = make_simple_element(1, Command::EnterSwordfight, Some(owner));
        element.priority = SequencePriority::PostponeEverythingButInjuries;
        let sequence = mgr.launch_element(element);
        mgr.postpone_element(sequence, 0);
        mgr.get_element_mut(tail, 0).unwrap().cross_postponed = Some((sequence, 0));
        tail = sequence;
        chain.push(sequence);

        mgr.stop_owner_current_from_root(
            owner,
            Some((root, 0)),
            SequencePriority::Preference,
            &|element| element.priority,
        );
    }

    assert_eq!(
        mgr.get_element(root, 0).unwrap().state,
        SequenceState::InProgress
    );
    assert!(chain.iter().all(|sequence| {
        mgr.get_element(*sequence, 0).unwrap().state == SequenceState::Postponed
    }));

    // The ceiling is only an admission shortcut. A newly linked weak tail
    // must disable it and remain observable to the exact Original traversal.
    let mut weak = make_simple_element(1, Command::Turn, Some(owner));
    weak.priority = SequencePriority::Normal;
    let weak = mgr.launch_element(weak);
    mgr.postpone_element(weak, 0);
    mgr.get_element_mut(tail, 0).unwrap().cross_postponed = Some((weak, 0));
    mgr.stop_owner_current_from_root(
        owner,
        Some((root, 0)),
        SequencePriority::Preference,
        &|element| element.priority,
    );
    assert_eq!(
        mgr.get_element(weak, 0).unwrap().state,
        SequenceState::Interrupted
    );
}

#[test]
fn strong_owner_summary_does_not_hide_weak_same_sequence_successor() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Soldier(crate::entity_id::SoldierId(7));
    let successor_owner = EntityId::Soldier(crate::entity_id::SoldierId(8));
    let mut sequence = Sequence::new();
    let mut root = make_simple_element(1, Command::QuitSwordfight, Some(owner));
    root.priority = SequencePriority::PostponeEverythingButInjuries;
    sequence.append_element(root);
    let mut successor = make_simple_element(2, Command::Turn, Some(successor_owner));
    successor.priority = SequencePriority::Normal;
    sequence.append_element(successor);
    let sequence = mgr.launch_sequence(sequence);

    mgr.stop_owner_current_from_root(
        owner,
        Some((sequence, 0)),
        SequencePriority::Preference,
        &|element| element.priority,
    );

    assert_eq!(
        mgr.get_element(sequence, 1).unwrap().state,
        SequenceState::Interrupted,
        "actor-wide priority shortcut must preserve Original's same-sequence recursion"
    );
}

#[test]
fn repeated_selected_stops_do_not_scan_unrelated_retained_sequences() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Soldier(crate::entity_id::SoldierId(7));
    let unrelated_owner = EntityId::Soldier(crate::entity_id::SoldierId(8));

    for _ in 0..4096 {
        mgr.launch_element(make_simple_element(1, Command::Turn, Some(unrelated_owner)));
    }
    let _ = mgr.hourglass();

    let mut roots = Vec::with_capacity(2048);
    for _ in 0..2048 {
        let mut element = make_simple_element(1, Command::EnterSwordfight, Some(owner));
        element.priority = SequencePriority::Normal;
        let sequence = mgr.launch_element(element);
        mgr.postpone_element(sequence, 0);
        roots.push(sequence);
    }

    for &sequence in &roots {
        mgr.stop_owner_current_from_root(
            owner,
            Some((sequence, 0)),
            SequencePriority::Preference,
            &|element| element.priority,
        );
    }

    for sequence in roots {
        assert_eq!(
            mgr.get_element(sequence, 0).unwrap().state,
            SequenceState::Interrupted
        );
    }
}

#[test]
fn stop_pending_matching_batches_terminal_link_cleanup() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Soldier(crate::entity_id::SoldierId(7));
    let unrelated_owner = EntityId::Soldier(crate::entity_id::SoldierId(8));

    for _ in 0..4096 {
        mgr.launch_element(make_simple_element(1, Command::Wait, Some(unrelated_owner)));
    }

    let mut matching = Vec::with_capacity(4096);
    for _ in 0..4096 {
        let mut element = make_simple_element(1, Command::ShootBow, Some(owner));
        element.priority = SequencePriority::Normal;
        matching.push(mgr.launch_element(element));
    }

    assert_eq!(
        mgr.stop_pending_elements_matching(
            owner,
            Command::ShootBow,
            SequencePriority::Preference,
            &|element| element.priority,
        ),
        matching.len(),
    );
    for sequence in matching {
        assert_eq!(
            mgr.get_element(sequence, 0).unwrap().state,
            SequenceState::Interrupted
        );
    }

    // EnterSwordfight performs this check for every queued entry even when
    // there is no bow work left. Keep the retained manager large enough that
    // an accidental all-sequence terminal-link cleanup per no-op call is
    // immediately visible in this stress regression.
    for _ in 0..4096 {
        assert_eq!(
            mgr.stop_pending_elements_matching(
                owner,
                Command::ShootBow,
                SequencePriority::Preference,
                &|element| element.priority,
            ),
            0,
        );
    }
}

#[test]
fn stop_pending_roots_do_not_scan_unrelated_retained_sequences() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Soldier(crate::entity_id::SoldierId(7));
    let unrelated_owner = EntityId::Soldier(crate::entity_id::SoldierId(8));

    for _ in 0..4096 {
        mgr.launch_element(make_simple_element(1, Command::Turn, Some(unrelated_owner)));
    }
    let _ = mgr.hourglass();

    let mut roots = Vec::with_capacity(2048);
    for _ in 0..2048 {
        let mut element = make_simple_element(1, Command::EnterSwordfight, Some(owner));
        element.priority = SequencePriority::Normal;
        roots.push(mgr.launch_element(element));
    }

    for &sequence in &roots {
        mgr.stop_pending_element_from_root(
            owner,
            (sequence, 0),
            SequencePriority::Preference,
            &|element| element.priority,
        );
    }
    mgr.compact_terminal_elements_to_go();

    for sequence in roots {
        assert_eq!(
            mgr.get_element(sequence, 0).unwrap().state,
            SequenceState::Interrupted
        );
    }
}

#[test]
fn stop_owner_walks_nested_cross_postponed_graph() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Soldier(crate::entity_id::SoldierId(8));

    let mut deepest = make_simple_element(1, Command::ParrySword, Some(owner));
    deepest.priority = SequencePriority::Preference;
    let deepest_seq = mgr.launch_element(deepest);
    mgr.postpone_element(deepest_seq, 0);

    let mut middle = make_simple_element(1, Command::EnterSwordfight, Some(owner));
    middle.priority = SequencePriority::Preference;
    let middle_seq = mgr.launch_element(middle);
    mgr.postpone_element(middle_seq, 0);
    mgr.get_element_mut(middle_seq, 0).unwrap().cross_postponed = Some((deepest_seq, 0));

    let mut injury = make_simple_element(1, Command::ReceiveSwordDamage, Some(owner));
    injury.priority = SequencePriority::Injury;
    let injury_seq = mgr.launch_element(injury);
    let _ = mgr.hourglass();
    mgr.element_in_progress(injury_seq, 0);
    mgr.get_element_mut(injury_seq, 0).unwrap().cross_postponed = Some((middle_seq, 0));

    mgr.stop_owner(owner, SequencePriority::Preference, &|element| {
        element.priority
    });

    assert_eq!(
        mgr.get_element(injury_seq, 0).unwrap().state,
        SequenceState::InProgress
    );
    for sequence in [middle_seq, deepest_seq] {
        assert_eq!(
            mgr.get_element(sequence, 0).unwrap().state,
            SequenceState::Interrupted,
            "every recursively postponed actor action must be stopped"
        );
    }
    assert_eq!(
        mgr.get_element(injury_seq, 0).unwrap().cross_postponed,
        None
    );
    assert_eq!(
        mgr.get_element(middle_seq, 0).unwrap().cross_postponed,
        None
    );
}

#[test]
fn stop_owner_walks_postponed_graph_from_pending_strong_blocker() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Soldier(crate::entity_id::SoldierId(8));

    let mut turn = make_simple_element(1, Command::Turn, Some(owner));
    turn.priority = SequencePriority::Normal;
    let turn_seq = mgr.launch_element(turn);
    mgr.postpone_element(turn_seq, 0);

    let mut attentive = make_simple_element(1, Command::EnterAttentiveMode, Some(owner));
    attentive.priority = SequencePriority::PostponeEverythingButInjuries;
    let attentive_seq = mgr.launch_element(attentive);
    mgr.postpone_element(attentive_seq, 0);
    mgr.get_element_mut(attentive_seq, 0)
        .unwrap()
        .cross_postponed = Some((turn_seq, 0));

    let mut leave_attentive = make_simple_element(1, Command::LeaveAttentiveMode, Some(owner));
    leave_attentive.priority = SequencePriority::PostponeEverythingButInjuries;
    let leave_seq = mgr.launch_element(leave_attentive);
    mgr.get_element_mut(leave_seq, 0).unwrap().cross_postponed = Some((attentive_seq, 0));

    assert_eq!(
        mgr.current_element_for_actor(owner),
        None,
        "Todo manager entries are not the actor's current element"
    );

    mgr.stop_owner(owner, SequencePriority::Preference, &|element| {
        element.priority
    });

    for sequence in [leave_seq, attentive_seq] {
        assert_ne!(
            mgr.get_element(sequence, 0).unwrap().state,
            SequenceState::Interrupted,
            "StopAll must preserve attentive-mode blockers"
        );
    }
    assert_eq!(
        mgr.get_element(turn_seq, 0).unwrap().state,
        SequenceState::Interrupted,
        "Original Stop follows a pending blocker's postponed pointer even when the blocker is too strong to stop"
    );
    assert_eq!(
        mgr.get_element(attentive_seq, 0).unwrap().cross_postponed,
        None
    );
}

#[test]
fn stop_owner_does_not_scan_unselected_postponed_branches() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Pc(crate::entity_id::PcId(3));

    // This branch belongs to the actor but is not reachable from the
    // actor's authoritative current element. Original Actor::Stop never
    // discovers it by scanning sequence ownership.
    let mut stale = make_simple_element(1, Command::EquipBow, Some(owner));
    stale.priority = SequencePriority::Preference;
    let stale_seq = mgr.launch_element(stale);
    let _ = mgr.hourglass();
    mgr.element_in_progress(stale_seq, 0);
    mgr.postpone_element(stale_seq, 0);

    let mut current = make_simple_element(1, Command::UnequipBow, Some(owner));
    current.priority = SequencePriority::Preference;
    let current_seq = mgr.launch_element(current);
    let _ = mgr.hourglass();
    mgr.element_in_progress(current_seq, 0);

    mgr.stop_owner(owner, SequencePriority::Preference, &|elem| elem.priority);

    assert_eq!(
        mgr.get_element(current_seq, 0).unwrap().state,
        SequenceState::Interrupted,
        "Actor::Stop must stop its selected mpSequenceElement"
    );
    assert_eq!(
        mgr.get_element(stale_seq, 0).unwrap().state,
        SequenceState::Postponed,
        "unlinked postponed ownership is not an Original traversal root"
    );
}

#[test]
fn postpone_element_consumes_its_existing_manager_registration() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Pc(crate::entity_id::PcId(3));
    let sequence_id = mgr.launch_element(make_simple_element(1, Command::EquipBow, Some(owner)));

    mgr.postpone_element(sequence_id, 0);

    assert!(
        mgr.hourglass().is_empty(),
        "Postpone runs after Original's manager pop and must consume Rust's eager registration"
    );
    assert_eq!(
        mgr.get_element(sequence_id, 0).unwrap().state,
        SequenceState::Postponed
    );
}

#[test]
fn stop_pending_elements_matching_clears_cross_postponed_shoot_bow() {
    let mut mgr = SequenceManager::new();
    let owner = EntityId::Pc(crate::entity_id::PcId(0));

    let mut current = make_simple_element(1, Command::ShootBow, Some(owner));
    current.priority = SequencePriority::Preference;
    current
        .orders
        .push_back(Order::test_new(OrderType::ShootingWithBow, 10.0, 0.0));
    let current_seq = mgr.launch_element(current);
    mgr.element_in_progress(current_seq, 0);

    let mut queued = make_simple_element(1, Command::ShootBow, Some(owner));
    queued.priority = SequencePriority::Preference;
    let queued_seq = mgr.launch_element(queued);

    mgr.get_element_mut(current_seq, 0).unwrap().cross_postponed = Some((queued_seq, 0));
    mgr.postpone_element(queued_seq, 0);

    assert!(mgr.queued_element_exists(owner, Command::ShootBow));

    let resolver = |_elem: &SequenceElement| SequencePriority::Preference;
    let stopped = mgr.stop_pending_elements_matching(
        owner,
        Command::ShootBow,
        SequencePriority::Preference,
        &resolver,
    );

    assert_eq!(stopped, 1);
    assert_eq!(
        mgr.get_element(queued_seq, 0).unwrap().state,
        SequenceState::Interrupted
    );
    assert_eq!(
        mgr.get_element(current_seq, 0).unwrap().cross_postponed,
        None
    );
    assert!(!mgr.queued_element_exists(owner, Command::ShootBow));
}

#[test]
fn manager_friday_evening_cleanup() {
    let mut mgr = SequenceManager::new();

    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    let seq_id = mgr.launch_sequence(seq);

    assert_eq!(mgr.sequence_count(), 1);

    // Mark element as terminated
    mgr.element_in_progress(seq_id, 0);
    mgr.element_terminated(seq_id, 0);

    // Now cleanup should remove it
    mgr.friday_evening_cleanup();
    assert_eq!(mgr.sequence_count(), 0);
}

#[test]
fn manager_terminate_sequence() {
    let mut mgr = SequenceManager::new();

    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    seq.append_element(make_simple_element(
        2,
        Command::Turn,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    let seq_id = mgr.launch_sequence(seq);

    assert!(mgr.terminate_sequence(seq_id));

    // Both elements should be interrupted
    let s = mgr.get_sequence(seq_id).unwrap();
    assert_eq!(s.elements[0].state, SequenceState::Interrupted);
}

#[test]
fn manager_immediate_commands() {
    let mut mgr = SequenceManager::new();

    let mut seq = Sequence::new();
    // LockUser executes immediately via engine
    seq.append_element(make_simple_element(1, Command::LockUser, None));
    let _seq_id = mgr.launch_sequence(seq);

    let actions = mgr.hourglass();
    assert_eq!(actions.len(), 1);
    assert!(matches!(
        actions[0],
        SequenceAction::ExecuteImmediateEngine { .. }
    ));
}

/// Immediate-class commands must land on `pending_immediate_actions`
/// synchronously inside `launch_sequence` so engine-side wrappers
/// can drain them this frame: registration = dispatch.  Only
/// non-immediate elements ever land on `elements_to_go`.
#[test]
fn manager_immediate_action_emitted_at_register_time() {
    let mut mgr = SequenceManager::new();
    assert!(!mgr.has_pending_immediate_actions());

    // Owner-only immediate (Speak) — must land on
    // `pending_immediate_actions` keyed to the owner.
    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Speak,
        Some(EntityId::Pc(crate::entity_id::PcId(7))),
    ));
    let seq_id = mgr.launch_sequence(seq);

    assert!(
        mgr.has_pending_immediate_actions(),
        "Speak should be queued onto pending_immediate_actions at launch_sequence time"
    );
    let actions = mgr.take_pending_immediate_actions();
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        SequenceAction::ExecuteImmediateOwner {
            owner,
            sequence_id,
            element_index,
        } => {
            assert_eq!(*owner, EntityId::Pc(crate::entity_id::PcId(7)));
            assert_eq!(*sequence_id, seq_id);
            assert_eq!(*element_index, 0);
        }
        other => panic!("expected ExecuteImmediateOwner for Speak, got {:?}", other),
    }
    assert!(!mgr.has_pending_immediate_actions());

    // Engine-only immediate (LockUser) — must land on
    // `pending_immediate_actions` regardless of owner.
    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(1, Command::LockUser, None));
    let _seq_id = mgr.launch_sequence(seq);
    let actions = mgr.take_pending_immediate_actions();
    assert_eq!(actions.len(), 1);
    assert!(matches!(
        actions[0],
        SequenceAction::ExecuteImmediateEngine { .. }
    ));

    // SendMessage with owner — owner branch. Original:
    // RHsequenceelement.cpp:765-774 invokes the owner callback during
    // registration rather than putting the command in Hourglass.
    let mut seq = Sequence::new();
    seq.append_element(SequenceElement::new_send_message(
        1,
        Some(EntityId::Pc(crate::entity_id::PcId(3))),
        SendMessageCommand::new(41, -2, 3),
    ));
    let _seq_id = mgr.launch_sequence(seq);
    let actions = mgr.take_pending_immediate_actions();
    assert!(matches!(
        actions[0],
        SequenceAction::ExecuteImmediateOwner {
            owner: EntityId::Pc(crate::entity_id::PcId(3)),
            ..
        }
    ));

    // SendMessage without owner — engine branch.
    let mut seq = Sequence::new();
    seq.append_element(SequenceElement::new_send_message(
        1,
        None,
        SendMessageCommand::new(42, 4, -5),
    ));
    let _seq_id = mgr.launch_sequence(seq);
    let actions = mgr.take_pending_immediate_actions();
    assert!(matches!(
        actions[0],
        SequenceAction::ExecuteImmediateEngine { .. }
    ));

    // Non-immediate command (Move) — must NOT land on the
    // immediate queue; only on `elements_to_go` for the next
    // hourglass.
    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    let _seq_id = mgr.launch_sequence(seq);
    assert!(!mgr.has_pending_immediate_actions());
    let actions = mgr.hourglass();
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, SequenceAction::InstructOwner { .. })),
        "Move should produce InstructOwner via the hourglass elements_to_go path"
    );
}

/// `hourglass` must drain `pending_immediate_actions` before
/// `elements_to_go` so immediate side effects land before
/// non-immediate dispatches in the same frame: registration =
/// dispatch, so the immediate fires synchronously while the
/// non-immediate is still being queued.
#[test]
fn manager_hourglass_drains_immediates_first() {
    let mut mgr = SequenceManager::new();

    // Mix one non-immediate and one immediate at the same level —
    // the non-immediate is registered first, but the immediate
    // should appear first in the action stream.
    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    seq.append_element(make_simple_element(1, Command::CameraJumpTo, None));
    let _seq_id = mgr.launch_sequence(seq);

    let actions = mgr.hourglass();
    assert!(actions.len() >= 2);
    assert!(
        matches!(actions[0], SequenceAction::ExecuteImmediateEngine { .. }),
        "first action should be the CameraJumpTo immediate, got {:?}",
        actions[0]
    );
}

#[test]
fn element_orders() {
    let mut elem = SequenceElement::new(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    );

    elem.push_order(Order::test_new(OrderType::WalkingUpright, 100.0, 200.0));
    elem.push_order(Order::test_new(OrderType::Turning, 150.0, 250.0));

    assert_eq!(elem.orders.len(), 2);
    assert_eq!(
        elem.current_order().unwrap().order_type,
        OrderType::WalkingUpright
    );
    assert_eq!(elem.next_order().unwrap().order_type, OrderType::Turning);

    // Proceed to next order
    let next = elem.proceed();
    assert!(next.is_some());
    assert_eq!(next.unwrap().order_type, OrderType::Turning);

    // Proceed past last
    let next = elem.proceed();
    assert!(next.is_none());
    assert!(elem.orders.is_empty());
}

#[test]
fn generic_element_properties() {
    let mut elem = SequenceElement::new_generic(
        1,
        Command::WaitTimer,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    );
    elem.set_property(Field::Timer, FieldValue::Integer(50));

    match elem.get_property(Field::Timer) {
        Some(FieldValue::Integer(50)) => {}
        other => panic!("expected Integer(50), got {:?}", other),
    }
}

#[test]
fn message_command_converts_legacy_fields_without_inventing_defaults() {
    let payload = SendMessageCommand::new(-17, 23, -42);
    let elem = SequenceElement::new_send_message(
        1,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
        payload,
    );

    assert_eq!(
        elem.sequence_command(),
        Ok(SequenceCommand::SendMessage(payload))
    );

    let mut missing = SequenceElement::new_generic(1, Command::SendMessage, None);
    missing.set_property(Field::Message, FieldValue::Integer(7));
    missing.set_property(Field::MessageArgument, FieldValue::Integer(8));
    assert_eq!(
        missing.sequence_command(),
        Err(SequenceInvariantError::MissingLegacyCommandField {
            command: Command::SendMessage,
            field: Field::MessageExtendedArgument,
        })
    );

    let wrong_subtype = SequenceElement::new(1, Command::SendMessage, None);
    assert_eq!(
        wrong_subtype.sequence_command(),
        Err(SequenceInvariantError::LegacyCommandRequiresGenericData {
            command: Command::SendMessage,
        })
    );
}

#[test]
fn checked_order_mutation_preserves_queue_on_invariant_errors() {
    let mut elem = SequenceElement::new(1, Command::Move, None);
    elem.push_order(Order::test_new(OrderType::WalkingUpright, 1.0, 2.0));

    assert_eq!(
        elem.try_push_order(Order::test_new(OrderType::Invalid, 3.0, 4.0)),
        Err(SequenceInvariantError::InvalidOrderAction)
    );
    assert_eq!(elem.orders.len(), 1);

    assert_eq!(
        elem.try_insert_order(2, Order::test_new(OrderType::Turning, 5.0, 6.0),),
        Err(SequenceInvariantError::OrderInsertionOutOfBounds { index: 2, len: 1 })
    );
    assert_eq!(elem.orders.len(), 1);
    assert_eq!(elem.orders[0].target_x, 1.0);
}

#[test]
fn checked_append_rejects_non_contiguous_command_levels() {
    let mut seq = Sequence::new();
    seq.append_element(SequenceElement::new(1, Command::Move, None));

    assert_eq!(
        seq.try_append_element(SequenceElement::new(3, Command::Turn, None)),
        Err(SequenceInvariantError::NonContiguousCommandLevel {
            previous: 1,
            next: 3,
        })
    );
    assert_eq!(seq.len(), 1);
}

#[test]
fn movement_element_speed_factor() {
    let mut elem = SequenceElement::new_movement(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
        OrderType::WalkingUpright,
    );
    assert_eq!(elem.speed_factor(), 1.0);

    elem.set_speed_factor(0.5);
    assert_eq!(elem.speed_factor(), 0.5);
}

#[test]
fn serde_roundtrip() {
    let mut seq = Sequence::new();
    seq.append_element(SequenceElement::new(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(5))),
    ));
    seq.append_element(SequenceElement::new_generic(1, Command::WaitTimer, None));
    seq.append_element(SequenceElement::new_movement(
        2,
        Command::PassDoor,
        Some(EntityId::Pc(crate::entity_id::PcId(1))),
        OrderType::WalkingUpright,
    ));

    let json = serde_json::to_string(&seq).unwrap();
    let back: Sequence = serde_json::from_str(&json).unwrap();

    assert_eq!(back.elements.len(), 3);
    assert_eq!(back.elements[0].command, Command::Move);
    assert_eq!(
        back.elements[0].owner,
        Some(EntityId::Pc(crate::entity_id::PcId(5)))
    );
    assert_eq!(back.elements[1].command, Command::WaitTimer);
    assert!(back.elements[1].data.is_generic());
    assert_eq!(back.elements[2].command, Command::PassDoor);
    assert!(back.elements[2].data.is_movement());
}

#[test]
fn parallel_elements_at_same_level() {
    let mut mgr = SequenceManager::new();

    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(1))),
    ));
    seq.append_element(make_simple_element(
        2,
        Command::Turn,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));

    let seq_id = mgr.launch_sequence(seq);

    // Should get two actions (both level-1 elements)
    let actions = mgr.hourglass();
    assert_eq!(actions.len(), 2);

    // Terminate both
    mgr.element_in_progress(seq_id, 0);
    mgr.element_in_progress(seq_id, 1);
    mgr.element_terminated(seq_id, 0);
    finish_test_condolations(&mut mgr);

    // Level 2 not yet started — one still running
    let actions = mgr.hourglass();
    assert!(actions.is_empty());

    mgr.element_terminated(seq_id, 1);
    finish_test_condolations(&mut mgr);

    // Now level 2 should start
    let actions = mgr.hourglass();
    assert_eq!(actions.len(), 1);
}

#[test]
fn element_about_to_be_launched() {
    let mut mgr = SequenceManager::new();

    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    let _seq_id = mgr.launch_sequence(seq);

    assert!(
        mgr.element_is_about_to_be_launched(EntityId::Pc(crate::entity_id::PcId(0)), Command::Move)
    );
    assert!(
        mgr.element_is_about_to_be_launched(EntityId::Pc(crate::entity_id::PcId(0)), Command::Null)
    );
    assert!(
        !mgr.element_is_about_to_be_launched(
            EntityId::Pc(crate::entity_id::PcId(1)),
            Command::Move
        )
    );
    assert!(
        !mgr.element_is_about_to_be_launched(
            EntityId::Pc(crate::entity_id::PcId(0)),
            Command::Turn
        )
    );
}

#[test]
fn pending_command_query_follows_only_current_elements_postponed_successor() {
    let owner = EntityId::Pc(crate::entity_id::PcId(0));
    let mut mgr = SequenceManager::new();

    let mut current_seq = Sequence::new();
    current_seq.append_element(make_simple_element(1, Command::Move, Some(owner)));
    let current_seq_id = mgr.launch_sequence(current_seq);
    mgr.element_in_progress(current_seq_id, 0);
    mgr.elements_to_go
        .retain(|&(seq_id, elem_idx)| (seq_id, elem_idx) != (current_seq_id, 0));

    let mut postponed_seq = Sequence::new();
    postponed_seq.append_element(make_simple_element(
        1,
        Command::EnterSwordfight,
        Some(owner),
    ));
    let postponed_seq_id = mgr.launch_sequence(postponed_seq);
    mgr.elements_to_go
        .retain(|&(seq_id, elem_idx)| (seq_id, elem_idx) != (postponed_seq_id, 0));
    mgr.postpone_element(postponed_seq_id, 0);

    // A postponed command elsewhere is not what Original's
    // current-element postponed pointer asks about.
    assert!(
        !mgr.element_is_about_to_be_launched_or_postponed_by_current(
            owner,
            Command::EnterSwordfight
        )
    );

    mgr.get_element_mut(current_seq_id, 0)
        .unwrap()
        .cross_postponed = Some((postponed_seq_id, 0));
    assert!(
        mgr.element_is_about_to_be_launched_or_postponed_by_current(
            owner,
            Command::EnterSwordfight
        )
    );
}

#[test]
fn pending_command_query_ignores_element_during_translation() {
    let owner = EntityId::Soldier(crate::entity_id::SoldierId(84));
    let mut mgr = SequenceManager::new();
    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::EnterSwordfight,
        Some(owner),
    ));
    let seq_id = mgr.launch_sequence(seq);

    // Hourglass removes the element from Original's launch list before
    // Go enters Actor::Instruct. The actor selection remains live during
    // Translate, but is not itself an about-to-launch command.
    mgr.elements_to_go.clear();
    mgr.set_translating_element(Some((owner, SequenceElementRef::new(seq_id, 0))));

    assert!(
        !mgr.element_is_about_to_be_launched_or_postponed_by_current(
            owner,
            Command::EnterSwordfight
        )
    );
    assert!(!mgr.element_is_about_to_be_launched_or_postponed_by_current(owner, Command::Null));
    assert!(!mgr.element_is_about_to_be_launched_or_postponed_by_current(owner, Command::Move));
}

#[test]
fn cancel_pending_move_commands() {
    let mut mgr = SequenceManager::new();

    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    seq.append_element(make_simple_element(
        1,
        Command::Turn,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    ));
    let _seq_id = mgr.launch_sequence(seq);

    mgr.cancel_pending_move_commands(EntityId::Pc(crate::entity_id::PcId(0)));

    // Only Turn should remain (Move was cancelled)
    let actions = mgr.hourglass();
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        SequenceAction::InstructOwner { element_index, .. } => assert_eq!(*element_index, 1),
        other => panic!("expected element 1, got {:?}", other),
    }
}

// ──────────────────────────────────────────────────────────
//  Movement transition rewriters
// ──────────────────────────────────────────────────────────

fn movement_elem(owner: EntityId, action: OrderType) -> SequenceElement {
    SequenceElement::new_movement(1, Command::Move, Some(owner), action)
}

fn movement_action(element: &SequenceElement) -> OrderType {
    let SequenceElementData::Movement { action, .. } = &element.data else {
        panic!("movement variant");
    };
    *action
}

fn loaded_v48_state(next: Option<SequenceElementRef>) -> LegacyV48SequenceElementState {
    LegacyV48SequenceElementState {
        deleted: false,
        script_driven: false,
        raw_dormant_posture_after_transition: None,
        raw_dormant_action_state_after_transition: None,
        next,
        postponed: None,
        mummy: None,
        linked_seek: None,
        damage_arrow: None,
        raw_sword_strike: None,
        raw_dormant_movement_action: None,
        order_state: Vec::new(),
        generic_raw_unions: Vec::new(),
    }
}

#[test]
fn make_fast_rewrites_walking_orders_to_running() {
    let mut elem = movement_elem(
        EntityId::Pc(crate::entity_id::PcId(0)),
        OrderType::WalkingUpright,
    );
    elem.state = SequenceState::InProgress;
    elem.push_order(Order::test_new(OrderType::WalkingUpright, 0.0, 0.0));
    elem.push_order(Order::test_new(
        OrderType::TransitionWaitingUprightWalkingUpright,
        0.0,
        0.0,
    ));
    elem.push_order(Order::test_new(
        OrderType::TransitionWalkingUprightWaitingUpright,
        0.0,
        0.0,
    ));

    make_fast_element(&mut elem);

    let SequenceElementData::Movement { flags, action, .. } = &elem.data else {
        panic!("movement variant");
    };
    assert!(flags.contains(MoveFlags::FAST));
    assert_eq!(*action, OrderType::RunningUpright);
    for o in &elem.orders {
        assert_eq!(o.order_type, OrderType::RunningUpright);
    }
}

#[test]
fn make_fast_preserves_unrelated_orders() {
    let mut elem = movement_elem(
        EntityId::Pc(crate::entity_id::PcId(0)),
        OrderType::WalkingUpright,
    );
    elem.state = SequenceState::InProgress;
    elem.push_order(Order::test_new(OrderType::Turning, 0.0, 0.0));
    elem.push_order(Order::test_new(OrderType::WalkingWithSword, 0.0, 0.0));

    make_fast_element(&mut elem);

    assert_eq!(elem.orders[0].order_type, OrderType::Turning);
    assert_eq!(elem.orders[1].order_type, OrderType::RunningWithSword);
}

#[test]
fn make_fast_rewrites_only_the_selected_elements_linked_chain() {
    let owner = EntityId::Pc(crate::entity_id::PcId(0));
    let mut mgr = SequenceManager::new();

    let mut selected = Sequence::new();
    selected.append_element(movement_elem(owner, OrderType::WalkingUpright));
    selected.append_element(movement_elem(owner, OrderType::WalkingUpright));
    let selected_id = mgr.launch_sequence(selected);
    mgr.element_in_progress(selected_id, 0);

    let mut unrelated = Sequence::new();
    unrelated.append_element(movement_elem(owner, OrderType::WalkingUpright));
    let unrelated_id = mgr.launch_sequence(unrelated);

    mgr.make_fast(owner);

    for idx in 0..2 {
        let SequenceElementData::Movement { action, flags, .. } = &mgr
            .get_element(selected_id, idx)
            .expect("selected chain element remains present")
            .data
        else {
            panic!("movement variant");
        };
        assert_eq!(*action, OrderType::RunningUpright);
        assert!(flags.contains(MoveFlags::FAST));
    }

    let SequenceElementData::Movement { action, flags, .. } = &mgr
        .get_element(unrelated_id, 0)
        .expect("unrelated sequence remains present")
        .data
    else {
        panic!("movement variant");
    };
    assert_eq!(*action, OrderType::WalkingUpright);
    assert!(!flags.contains(MoveFlags::FAST));
}

#[test]
fn make_fast_rewrites_a_terminal_same_owner_follower() {
    let owner = EntityId::Pc(crate::entity_id::PcId(0));
    let mut mgr = SequenceManager::new();
    let mut sequence = Sequence::new();
    sequence.append_element(movement_elem(owner, OrderType::WalkingUpright));
    let mut finished = movement_elem(owner, OrderType::WalkingUpright);
    finished.state = SequenceState::Terminated;
    finished.push_order(Order::test_new(OrderType::WalkingUpright, 0.0, 0.0));
    sequence.append_element(finished);
    let sequence_id = mgr.launch_sequence(sequence);
    mgr.element_in_progress(sequence_id, 0);

    mgr.make_fast(owner);

    let follower = mgr
        .get_element(sequence_id, 1)
        .expect("terminal following element remains present");
    let SequenceElementData::Movement { action, flags, .. } = &follower.data else {
        panic!("movement variant");
    };
    assert_eq!(*action, OrderType::RunningUpright);
    assert!(flags.contains(MoveFlags::FAST));
    assert_eq!(follower.orders[0].order_type, OrderType::RunningUpright);
}

#[test]
fn make_fast_rewrites_materialized_orders_only_after_todo() {
    let owner = EntityId::Pc(crate::entity_id::PcId(0));
    let mut todo = movement_elem(owner, OrderType::WalkingUpright);
    todo.push_order(Order::test_new(OrderType::WalkingUpright, 0.0, 0.0));
    let mut in_progress = todo.clone();
    in_progress.state = SequenceState::InProgress;

    make_fast_element(&mut todo);
    make_fast_element(&mut in_progress);

    for element in [&todo, &in_progress] {
        let SequenceElementData::Movement { action, flags, .. } = &element.data else {
            panic!("movement variant");
        };
        assert_eq!(*action, OrderType::RunningUpright);
        assert!(flags.contains(MoveFlags::FAST));
    }
    assert_eq!(todo.orders[0].order_type, OrderType::WalkingUpright);
    assert_eq!(in_progress.orders[0].order_type, OrderType::RunningUpright);
}

#[test]
fn make_slow_is_symmetric_to_make_fast() {
    let mut elem = movement_elem(
        EntityId::Pc(crate::entity_id::PcId(0)),
        OrderType::RunningUpright,
    );
    elem.state = SequenceState::InProgress;
    if let SequenceElementData::Movement { flags, .. } = &mut elem.data {
        *flags |= MoveFlags::FAST;
    }
    elem.push_order(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
    elem.push_order(Order::test_new(OrderType::RunningWithSword, 0.0, 0.0));
    elem.push_order(Order::test_new(
        OrderType::TransitionWaitingUprightRunningUpright,
        0.0,
        0.0,
    ));
    elem.push_order(Order::test_new(
        OrderType::TransitionRunningUprightWaitingUpright,
        0.0,
        0.0,
    ));

    make_slow_element(&mut elem);

    let SequenceElementData::Movement { flags, action, .. } = &elem.data else {
        panic!("movement variant");
    };
    assert!(!flags.contains(MoveFlags::FAST));
    assert_eq!(*action, OrderType::WalkingUpright);
    assert_eq!(elem.orders[0].order_type, OrderType::WalkingUpright);
    assert_eq!(elem.orders[1].order_type, OrderType::WalkingWithSword);
    assert_eq!(elem.orders[2].order_type, OrderType::WalkingUpright);
    assert_eq!(elem.orders[3].order_type, OrderType::WalkingUpright);
}

#[test]
fn make_upright_rewrites_crouched_orders() {
    let mut elem = movement_elem(
        EntityId::Pc(crate::entity_id::PcId(0)),
        OrderType::WalkingCrouched,
    );
    elem.state = SequenceState::InProgress;
    elem.push_order(Order::test_new(OrderType::WalkingCrouched, 0.0, 0.0));
    elem.push_order(Order::test_new(
        OrderType::TransitionWaitingCrouchedWalkingCrouched,
        0.0,
        0.0,
    ));
    elem.push_order(Order::test_new(
        OrderType::TransitionWalkingCrouchedWaitingCrouched,
        0.0,
        0.0,
    ));

    make_upright_element(&mut elem);

    let SequenceElementData::Movement { action, .. } = &elem.data else {
        panic!("movement variant");
    };
    assert_eq!(*action, OrderType::WalkingUpright);
    for o in &elem.orders {
        assert_eq!(o.order_type, OrderType::WalkingUpright);
    }
}

#[test]
fn make_upright_cancels_pending_crouch_down() {
    let mut elem = SequenceElement::new(
        1,
        Command::CrouchDown,
        Some(EntityId::Pc(crate::entity_id::PcId(0))),
    );
    make_upright_element(&mut elem);
    assert_eq!(elem.command, Command::Null);
}

#[test]
fn make_crouched_rewrites_upright_orders_and_clears_fast() {
    let mut elem = movement_elem(
        EntityId::Pc(crate::entity_id::PcId(0)),
        OrderType::RunningUpright,
    );
    elem.state = SequenceState::InProgress;
    if let SequenceElementData::Movement { flags, .. } = &mut elem.data {
        *flags |= MoveFlags::FAST;
    }
    elem.push_order(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
    elem.push_order(Order::test_new(
        OrderType::TransitionWaitingUprightWalkingUpright,
        0.0,
        0.0,
    ));
    elem.push_order(Order::test_new(
        OrderType::TransitionRunningUprightWaitingUpright,
        0.0,
        0.0,
    ));

    make_crouched_element(&mut elem);

    let SequenceElementData::Movement { flags, action, .. } = &elem.data else {
        panic!("movement variant");
    };
    assert!(!flags.contains(MoveFlags::FAST));
    assert_eq!(*action, OrderType::WalkingCrouched);
    for o in &elem.orders {
        assert_eq!(o.order_type, OrderType::WalkingCrouched);
    }
}

#[test]
fn set_action_recursive_walks_sequence() {
    let mut mgr = SequenceManager::new();
    let mut seq = Sequence::new();
    seq.append_element(SequenceElement::new_movement(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(1))),
        OrderType::RunningUpright,
    ));
    seq.append_element(SequenceElement::new_movement(
        2,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(1))),
        OrderType::RunningUpright,
    ));
    // Different owner — should terminate the walk.
    seq.append_element(SequenceElement::new_movement(
        3,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(2))),
        OrderType::RunningUpright,
    ));
    let seq_id = mgr.launch_sequence(seq);

    mgr.set_action_recursive(seq_id, 0, OrderType::WalkingCrouched);

    let s = mgr.get_sequence(seq_id).unwrap();
    for i in 0..2 {
        let SequenceElementData::Movement { action, .. } = s.elements[i].data else {
            panic!("movement variant");
        };
        assert_eq!(action, OrderType::WalkingCrouched);
    }
    // Third element's owner differs — untouched.
    let SequenceElementData::Movement { action, .. } = s.elements[2].data else {
        panic!("movement variant");
    };
    assert_eq!(action, OrderType::RunningUpright);
}

#[test]
fn set_action_recursive_honors_loaded_null_and_nonadjacent_next() {
    let owner = EntityId::Pc(crate::entity_id::PcId(1));

    let mut null_mgr = SequenceManager::new();
    let mut null_sequence = Sequence::new();
    null_sequence.append_element(movement_elem(owner, OrderType::RunningUpright));
    null_sequence.append_element(movement_elem(owner, OrderType::RunningUpright));
    let null_id = null_mgr.launch_sequence(null_sequence);
    null_mgr.get_element_mut(null_id, 0).unwrap().legacy_v48 = Some(loaded_v48_state(None));
    null_mgr.set_action_recursive(null_id, 0, OrderType::WalkingCrouched);
    assert_eq!(
        movement_action(null_mgr.get_element(null_id, 0).unwrap()),
        OrderType::WalkingCrouched
    );
    assert_eq!(
        movement_action(null_mgr.get_element(null_id, 1).unwrap()),
        OrderType::RunningUpright
    );

    let mut linked_mgr = SequenceManager::new();
    let mut linked_sequence = Sequence::new();
    for _ in 0..3 {
        linked_sequence.append_element(movement_elem(owner, OrderType::RunningUpright));
    }
    let linked_id = linked_mgr.launch_sequence(linked_sequence);
    linked_mgr.get_element_mut(linked_id, 0).unwrap().legacy_v48 = Some(loaded_v48_state(Some(
        SequenceElementRef::new(linked_id, 2),
    )));
    linked_mgr.set_action_recursive(linked_id, 0, OrderType::WalkingCrouched);
    assert_eq!(
        movement_action(linked_mgr.get_element(linked_id, 0).unwrap()),
        OrderType::WalkingCrouched
    );
    assert_eq!(
        movement_action(linked_mgr.get_element(linked_id, 1).unwrap()),
        OrderType::RunningUpright
    );
    assert_eq!(
        movement_action(linked_mgr.get_element(linked_id, 2).unwrap()),
        OrderType::WalkingCrouched
    );
}

#[test]
fn set_action_recursive_follows_cross_postponed_link() {
    let owner = EntityId::Pc(crate::entity_id::PcId(1));
    let mut mgr = SequenceManager::new();
    let root = mgr.launch_element(movement_elem(owner, OrderType::RunningUpright));
    let postponed = mgr.launch_element(movement_elem(owner, OrderType::RunningUpright));
    mgr.get_element_mut(root, 0).unwrap().cross_postponed = Some((postponed, 0));

    mgr.set_action_recursive(root, 0, OrderType::WalkingCrouched);

    assert_eq!(
        movement_action(mgr.get_element(postponed, 0).unwrap()),
        OrderType::WalkingCrouched
    );
}

#[test]
fn loaded_nonadjacent_next_controls_interruption_cascade() {
    let owner = EntityId::Pc(crate::entity_id::PcId(1));
    let mut mgr = SequenceManager::new();
    let mut sequence = Sequence::new();
    for _ in 0..3 {
        sequence.append_element(SequenceElement::new(1, Command::Generic, Some(owner)));
    }
    let sequence_id = mgr.launch_sequence(sequence);
    mgr.get_element_mut(sequence_id, 0).unwrap().legacy_v48 = Some(loaded_v48_state(Some(
        SequenceElementRef::new(sequence_id, 2),
    )));

    mgr.element_interrupted(sequence_id, 0, CascadeFlags::FOLLOWING);
    // The owner's condolence card is a synchronous boundary; the
    // cascade only continues once the card's Think has completed.
    let mut pending = mgr.drain_pending_condolations();
    assert_eq!(pending.len(), 1);
    mgr.finish_pending_condolation(pending.remove(0));

    assert_eq!(
        mgr.get_element(sequence_id, 1).unwrap().state,
        SequenceState::Todo
    );
    assert_eq!(
        mgr.get_element(sequence_id, 2).unwrap().state,
        SequenceState::Interrupted
    );
}

#[test]
fn loaded_movement_interruption_reaches_cross_sequence_linked_seek() {
    let owner = EntityId::Pc(crate::entity_id::PcId(1));
    let mut mgr = SequenceManager::new();

    let mut linked = Sequence::new();
    linked.append_element(movement_elem(owner, OrderType::WalkingUpright));
    let linked_id = mgr.launch_sequence(linked);

    let mut movement = movement_elem(owner, OrderType::WalkingUpright);
    movement.command = Command::MoveWaiting;
    let movement_id = mgr.launch_element(movement);
    mgr.get_element_mut(movement_id, 0).unwrap().legacy_v48 = Some(LegacyV48SequenceElementState {
        linked_seek: Some(Some(SequenceElementRef::new(linked_id, 0))),
        ..loaded_v48_state(None)
    });

    mgr.element_interrupted(movement_id, 0, CascadeFlags::NEXT_LEVEL);

    assert_eq!(
        mgr.get_element(movement_id, 0).unwrap().command,
        Command::Move,
        "MaybeCancelPathRequest restores MOVE_WAITING to MOVE"
    );
    assert_eq!(
        mgr.get_element(linked_id, 0).unwrap().state,
        SequenceState::Interrupted,
        "movement override interrupts its exact loaded linked Seek"
    );
    let cards = mgr.drain_pending_condolations();
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[0].card.seq_id, linked_id);
    assert_eq!(cards[1].card.seq_id, movement_id);
    assert_eq!(cards[0].card.cancel_path_request_owner, Some(owner));
    assert_eq!(cards[1].card.cancel_path_request_owner, None);
}

#[test]
fn loaded_nonadjacent_next_controls_stop_recursion() {
    let owner = EntityId::Pc(crate::entity_id::PcId(1));
    let mut mgr = SequenceManager::new();
    let mut sequence = Sequence::new();
    for priority in [
        SequencePriority::NonInterruptable,
        SequencePriority::Normal,
        SequencePriority::Normal,
    ] {
        let mut element = SequenceElement::new(1, Command::Generic, Some(owner));
        element.priority = priority;
        sequence.append_element(element);
    }
    let sequence_id = mgr.launch_sequence(sequence);
    mgr.get_element_mut(sequence_id, 0).unwrap().legacy_v48 = Some(loaded_v48_state(Some(
        SequenceElementRef::new(sequence_id, 2),
    )));

    mgr.get_sequence_mut(sequence_id)
        .unwrap()
        .stop_element(0, SequencePriority::Normal, &|_| SequencePriority::Normal);

    assert_eq!(
        mgr.get_element(sequence_id, 1).unwrap().state,
        SequenceState::Todo
    );
    assert_eq!(
        mgr.get_element(sequence_id, 2).unwrap().state,
        SequenceState::Interrupted
    );
    assert_eq!(
        mgr.get_element(sequence_id, 0)
            .unwrap()
            .legacy_v48
            .as_ref()
            .unwrap()
            .next,
        None,
        "Original clears mpsqeNextSequenceElement after Stop interrupts it"
    );
}

#[test]
fn stop_movement_rewrites_order_and_shortens_only_element_destination() {
    let mut mgr = SequenceManager::new();
    let mut seq = Sequence::new();
    let mut elem = SequenceElement::new_movement(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(1))),
        OrderType::WalkingUpright,
    );
    elem.push_order(Order::test_new(OrderType::WalkingUpright, 100.0, 0.0));
    elem.push_order(Order::test_new(OrderType::WalkingUpright, 200.0, 0.0));
    seq.append_element(elem);
    let seq_id = mgr.launch_sequence(seq);

    // Advance to InProgress so stop_movement_for_owner applies.
    let _ = mgr.hourglass();
    mgr.element_in_progress(seq_id, 0);

    let mut cancellations: Vec<EntityId> = Vec::new();
    let mut next_order_id = 1u32;
    let changed = mgr.stop_movement_for_owner(
        EntityId::Pc(crate::entity_id::PcId(1)),
        crate::coordinates::MapPoint { x: 0.0, y: 0.0 },
        SequencePriority::NonInterruptable,
        &|_| SequencePriority::Normal,
        &mut next_order_id,
        &mut |id| cancellations.push(id),
    );
    assert!(changed);
    let s = mgr.get_sequence(seq_id).unwrap();
    let first = s.elements[0].current_order().unwrap();
    assert_eq!(
        first.order_type,
        OrderType::TransitionWalkingUprightWaitingUpright
    );
    assert_eq!(first.target_x, 100.0);
    let SequenceElementData::Movement { destination, .. } = &s.elements[0].data else {
        panic!("movement data");
    };
    assert!(destination.x <= 10.0 + 0.001);
    // Trailing order should have been dropped.
    assert_eq!(s.elements[0].orders.len(), 1);
    assert!(cancellations.is_empty()); // No MoveWaiting — no cancellation.

    // The generic half of Actor::Stop runs after StopMovement. Original
    // leaves this rewritten movement InProgress so the transition can
    // play; only movement actions without a rewrite arm were interrupted
    // above.
    mgr.stop_owner(
        EntityId::Pc(crate::entity_id::PcId(1)),
        SequencePriority::NonInterruptable,
        &|_| SequencePriority::Normal,
    );
    assert_eq!(
        mgr.get_element(seq_id, 0).unwrap().state,
        SequenceState::InProgress
    );
}

#[test]
fn stop_movement_rewrite_does_not_cancel_path_for_move_waiting() {
    // Path cancellation only fires when the element is pushed to
    // `Interrupted` (default switch branch).  A successful rewrite
    // keeps the element in INPROGRESS and the path request stays
    // alive.
    let mut mgr = SequenceManager::new();
    let mut seq = Sequence::new();
    let mut elem = SequenceElement::new_movement(
        1,
        Command::MoveWaiting,
        Some(EntityId::Pc(crate::entity_id::PcId(1))),
        OrderType::WalkingUpright,
    );
    elem.push_order(Order::test_new(OrderType::WalkingUpright, 100.0, 0.0));
    seq.append_element(elem);
    let seq_id = mgr.launch_sequence(seq);
    let _ = mgr.hourglass();
    mgr.element_in_progress(seq_id, 0);

    let mut cancellations: Vec<EntityId> = Vec::new();
    let mut next_order_id = 1u32;
    mgr.stop_movement_for_owner(
        EntityId::Pc(crate::entity_id::PcId(1)),
        crate::coordinates::MapPoint::default(),
        SequencePriority::NonInterruptable,
        &|_| SequencePriority::Normal,
        &mut next_order_id,
        &mut |id| cancellations.push(id),
    );
    assert!(cancellations.is_empty());
    let s = mgr.get_sequence(seq_id).unwrap();
    assert_eq!(s.elements[0].command, Command::MoveWaiting);
}

#[test]
fn stop_movement_cancels_path_on_interrupt() {
    // With an action that has no waiting-transition variant, the
    // element falls into the default branch and gets interrupted;
    // a MoveWaiting command goes through path cancellation.
    let mut mgr = SequenceManager::new();
    let mut seq = Sequence::new();
    let mut elem = SequenceElement::new_movement(
        1,
        Command::MoveWaiting,
        Some(EntityId::Pc(crate::entity_id::PcId(1))),
        OrderType::Turning,
    );
    elem.push_order(Order::test_new(OrderType::Turning, 100.0, 0.0));
    seq.append_element(elem);
    let seq_id = mgr.launch_sequence(seq);
    let _ = mgr.hourglass();
    mgr.element_in_progress(seq_id, 0);

    let mut cancellations: Vec<EntityId> = Vec::new();
    let mut next_order_id = 1u32;
    mgr.stop_movement_for_owner(
        EntityId::Pc(crate::entity_id::PcId(1)),
        crate::coordinates::MapPoint::default(),
        SequencePriority::NonInterruptable,
        &|_| SequencePriority::Normal,
        &mut next_order_id,
        &mut |id| cancellations.push(id),
    );
    assert_eq!(cancellations, vec![EntityId::Pc(crate::entity_id::PcId(1))]);
    let s = mgr.get_sequence(seq_id).unwrap();
    assert_eq!(s.elements[0].command, Command::Move);
    assert_eq!(s.elements[0].state, SequenceState::Interrupted);
}

#[test]
fn stop_movement_interrupts_element_with_unknown_action() {
    let mut mgr = SequenceManager::new();
    let mut seq = Sequence::new();
    let mut elem = SequenceElement::new_movement(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(1))),
        OrderType::Turning,
    );
    elem.push_order(Order::test_new(OrderType::Turning, 100.0, 0.0));
    seq.append_element(elem);
    let seq_id = mgr.launch_sequence(seq);
    let _ = mgr.hourglass();
    mgr.element_in_progress(seq_id, 0);

    let mut cancellations: Vec<EntityId> = Vec::new();
    let mut next_order_id = 1u32;
    mgr.stop_movement_for_owner(
        EntityId::Pc(crate::entity_id::PcId(1)),
        crate::coordinates::MapPoint::default(),
        SequencePriority::NonInterruptable,
        &|_| SequencePriority::Normal,
        &mut next_order_id,
        &mut |id| cancellations.push(id),
    );
    let s = mgr.get_sequence(seq_id).unwrap();
    assert_eq!(s.elements[0].state, SequenceState::Interrupted);
}

#[test]
fn insert_transition_start_splits_long_walking_order() {
    let mut elem = movement_elem(
        EntityId::Pc(crate::entity_id::PcId(0)),
        OrderType::WalkingUpright,
    );
    // Single walking order 100 units along +x.
    elem.push_order(Order::test_new(OrderType::WalkingUpright, 100.0, 0.0));

    let mut next_order_id = 1u32;
    let inserted = elem.insert_transition_start(
        OrderType::TransitionWaitingUprightWalkingUpright,
        OrderType::WalkingUpright,
        10.0,
        crate::coordinates::MapPoint { x: 0.0, y: 0.0 },
        &mut next_order_id,
    );

    assert!(inserted);
    assert_eq!(elem.orders.len(), 2);
    assert_eq!(
        elem.orders[0].order_type,
        OrderType::TransitionWaitingUprightWalkingUpright
    );
    assert!((elem.orders[0].target_x - 10.0).abs() < 0.01);
    assert_eq!(elem.orders[1].order_type, OrderType::WalkingUpright);
}

#[test]
fn insert_transition_start_reports_short_order_relabel() {
    let mut elem = movement_elem(
        EntityId::Pc(crate::entity_id::PcId(0)),
        OrderType::WalkingUpright,
    );
    elem.push_order(Order::test_new(OrderType::WalkingUpright, 5.0, 0.0));

    let mut next_order_id = 1u32;
    let inserted = elem.insert_transition_start(
        OrderType::TransitionWaitingUprightWalkingUpright,
        OrderType::WalkingUpright,
        10.0,
        crate::coordinates::MapPoint { x: 0.0, y: 0.0 },
        &mut next_order_id,
    );

    assert!(
        inserted,
        "an in-place relabel is still a startup transition"
    );
    assert_eq!(elem.orders.len(), 1);
    assert_eq!(
        elem.orders[0].order_type,
        OrderType::TransitionWaitingUprightWalkingUpright
    );
}

#[test]
fn pop_current_order_shrinks_remaining_transition_prefix() {
    let mut elem = movement_elem(
        EntityId::Pc(crate::entity_id::PcId(0)),
        OrderType::WalkingUpright,
    );
    elem.push_order(Order::test_new(
        OrderType::TransitionWaitingCapeWaitingUpright,
        0.0,
        0.0,
    ));
    elem.push_order(Order::test_new(OrderType::WalkingUpright, 10.0, 0.0));
    elem.num_transition_orders = 1;

    elem.pop_current_order().expect("transition order");
    assert_eq!(elem.num_transition_orders, 0);
    assert_eq!(
        elem.current_order().map(|order| order.order_type),
        Some(OrderType::WalkingUpright)
    );
}

#[test]
fn insert_transition_end_appends_transition_before_last_order() {
    let mut elem = movement_elem(
        EntityId::Pc(crate::entity_id::PcId(0)),
        OrderType::WalkingUpright,
    );
    elem.push_order(Order::test_new(OrderType::WalkingUpright, 0.0, 0.0));
    let mut final_order = Order::test_new(OrderType::WalkingUpright, 100.0, 0.0);
    final_order.tolerance = 40.0;
    elem.push_order(final_order);
    if let SequenceElementData::Movement { tolerance, .. } = &mut elem.data {
        *tolerance = 40.0;
    }

    let mut next_order_id = 1u32;
    elem.insert_transition_end(
        OrderType::TransitionWalkingUprightWaitingUpright,
        OrderType::WalkingUpright,
        10.0,
        crate::coordinates::MapPoint { x: 0.0, y: 0.0 },
        1.0,
        &mut next_order_id,
    );

    // The last WalkingUpright order is relabelled to the transition,
    // and a new WalkingUpright order is inserted in front of it. The
    // 40-unit tolerance is added to the 10-unit transition distance, so
    // the inserted walking goal is 50 units back from the endpoint.
    assert_eq!(elem.orders.len(), 3);
    assert_eq!(elem.orders[0].order_type, OrderType::WalkingUpright);
    assert_eq!(elem.orders[1].order_type, OrderType::WalkingUpright);
    assert!((elem.orders[1].target_x - 50.0).abs() < 0.5);
    assert_eq!(
        elem.orders[2].order_type,
        OrderType::TransitionWalkingUprightWaitingUpright
    );
    assert_eq!(elem.orders[1].tolerance, 0.0);
    assert_eq!(
        elem.orders[2].tolerance, 40.0,
        "relabeling the final order preserves its existing arrival tolerance"
    );
}

/// nicouzouf Savegame_047 replay-004, frame 563: Soldier51's rider-charge
/// destination (see `ai_enemy::battle::rider_charge_goal_geometry`) is
/// spliced with the ~26-unit running→waiting stop transition from the
/// rider's position. The resulting RunningUpright order goal is the value
/// the Original trace records as `position_goal_map` at frame 564; a
/// one-ULP-lower destination Y (0x4425e9c8) lands the spliced goal at
/// 0x44230254 instead.
#[test]
fn insert_transition_end_matches_frame563_rider_charge_fixture() {
    let mut elem = movement_elem(
        EntityId::Soldier(crate::entity_id::SoldierId(51)),
        OrderType::RunningUpright,
    );
    elem.push_order(Order::test_new(
        OrderType::RunningUpright,
        f32::from_bits(0x442f_2b23),
        f32::from_bits(0x4425_e9c9),
    ));

    let mut next_order_id = 1u32;
    elem.insert_transition_end(
        OrderType::TransitionRunningUprightWaitingUpright,
        OrderType::RunningUpright,
        26.0,
        crate::coordinates::MapPoint {
            x: f32::from_bits(0x448f_3c66),
            y: f32::from_bits(0x43dc_a7ea),
        },
        1.0,
        &mut next_order_id,
    );

    assert_eq!(elem.orders.len(), 2);
    assert_eq!(elem.orders[0].order_type, OrderType::RunningUpright);
    assert_eq!(elem.orders[0].target_x.to_bits(), 0x4434_fbd2);
    assert_eq!(elem.orders[0].target_y.to_bits(), 0x4423_0255);
    assert_eq!(
        elem.orders[1].order_type,
        OrderType::TransitionRunningUprightWaitingUpright
    );
}

#[test]
fn cleanup_duplicate_orders_removes_consecutive_matches() {
    let mut elem = movement_elem(
        EntityId::Pc(crate::entity_id::PcId(0)),
        OrderType::WalkingUpright,
    );
    elem.push_order(Order::test_new(OrderType::WalkingUpright, 10.0, 10.0));
    elem.push_order(Order::test_new(OrderType::WalkingUpright, 10.0, 10.0));
    elem.push_order(Order::test_new(OrderType::WalkingUpright, 20.0, 20.0));
    elem.push_order(Order::test_new(OrderType::WalkingUpright, 20.0, 20.0));

    elem.cleanup_duplicate_orders();

    assert_eq!(elem.orders.len(), 2);
    assert_eq!(elem.orders[0].target_x, 10.0);
    assert_eq!(elem.orders[1].target_x, 20.0);
}

#[test]
fn is_next_movement_detects_same_owner_chain() {
    let mut mgr = SequenceManager::new();
    let mut seq = Sequence::new();
    seq.append_element(SequenceElement::new_movement(
        1,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(1))),
        OrderType::WalkingUpright,
    ));
    seq.append_element(SequenceElement::new_movement(
        2,
        Command::Move,
        Some(EntityId::Pc(crate::entity_id::PcId(1))),
        OrderType::WalkingUpright,
    ));
    seq.append_element(SequenceElement::new(
        3,
        Command::JumpCmd,
        Some(EntityId::Pc(crate::entity_id::PcId(1))),
    ));
    let seq_id = mgr.launch_sequence(seq);

    assert!(mgr.is_next_movement(seq_id, 0));
    assert!(!mgr.is_next_movement(seq_id, 1)); // next is Jump (Simple) — not movement
    assert!(mgr.is_next_movement_or_jump(seq_id, 1));
    assert!(!mgr.is_next_movement(seq_id, 2)); // last element — nothing next
}

#[test]
fn loaded_v48_null_next_overrides_physical_adjacency() {
    let owner = EntityId::Pc(crate::entity_id::PcId(1));
    let mut mgr = SequenceManager::new();
    let mut sequence = Sequence::new();
    sequence.append_element(SequenceElement::new(1, Command::Generic, Some(owner)));
    sequence.append_element(movement_elem(owner, OrderType::WalkingUpright));
    let sequence_id = mgr.launch_sequence(sequence);
    mgr.get_element_mut(sequence_id, 0)
        .expect("loaded first element exists")
        .legacy_v48 = Some(loaded_v48_state(None));

    assert!(!mgr.is_next_movement(sequence_id, 0));
    assert!(mgr.is_last_real_action(sequence_id, 0));
}

#[test]
fn loaded_v48_nonadjacent_next_is_authoritative() {
    let owner = EntityId::Pc(crate::entity_id::PcId(1));
    let mut mgr = SequenceManager::new();
    let mut sequence = Sequence::new();
    sequence.append_element(SequenceElement::new(1, Command::Generic, Some(owner)));
    sequence.append_element(SequenceElement::new(2, Command::Generic, Some(owner)));
    sequence.append_element(SequenceElement::new_movement(
        3,
        Command::Move,
        Some(owner),
        OrderType::WalkingUpright,
    ));
    let sequence_id = mgr.launch_sequence(sequence);
    mgr.get_element_mut(sequence_id, 0)
        .expect("loaded first element exists")
        .legacy_v48 = Some(loaded_v48_state(Some(SequenceElementRef::new(
        sequence_id,
        2,
    ))));

    assert!(mgr.is_next_movement(sequence_id, 0));
    assert!(!mgr.is_last_real_action(sequence_id, 0));
}

#[test]
fn last_real_action_checks_postponed_on_each_skipped_follower() {
    let owner = EntityId::Pc(crate::entity_id::PcId(1));
    for skipped_command in [Command::Wait, Command::AssertPosition] {
        let mut mgr = SequenceManager::new();
        let mut primary = Sequence::new();
        primary.append_element(SequenceElement::new(1, Command::Generic, Some(owner)));
        primary.append_element(SequenceElement::new(2, skipped_command, Some(owner)));
        let primary_id = mgr.launch_sequence(primary);

        let postponed_id =
            mgr.launch_element(SequenceElement::new(1, Command::Generic, Some(owner)));
        mgr.get_element_mut(primary_id, 1)
            .expect("skipped follower exists")
            .cross_postponed = Some((postponed_id, 0));

        assert!(
            !mgr.is_last_real_action(primary_id, 0),
            "{skipped_command:?} follower's postponed action must count as real"
        );
    }
}

#[test]
fn last_real_action_counts_following_manager_owned_element() {
    let first_owner = EntityId::Soldier(crate::entity_id::SoldierId(1));
    let mut mgr = SequenceManager::new();
    let mut sequence = Sequence::new();
    sequence.append_element(SequenceElement::new(1, Command::MoveOk, Some(first_owner)));
    sequence.append_element(SequenceElement::new(2, Command::Timer, None));
    let sequence_id = mgr.launch_sequence(sequence);

    assert!(
        !mgr.is_last_real_action(sequence_id, 0),
        "Original follows GetFollowingElement without an owner-identity gate"
    );
}

#[test]
fn last_real_action_stops_at_halt_severed_following_edge() {
    let owner = EntityId::Civilian(crate::entity_id::CivilianId(1));
    let mut mgr = SequenceManager::new();
    let mut sequence = Sequence::new();
    sequence.append_element(SequenceElement::new(1, Command::PassDoor, Some(owner)));
    sequence.append_element(SequenceElement::new(
        2,
        Command::AssertPosition,
        Some(owner),
    ));
    sequence.append_element(SequenceElement::new(3, Command::Move, Some(owner)));
    let sequence_id = mgr.launch_sequence(sequence);

    assert!(
        !mgr.is_last_real_action(sequence_id, 0),
        "an intact skipped AssertPosition edge must still expose the following Move"
    );
    mgr.get_element_mut(sequence_id, 0)
        .expect("PassDoor exists")
        .next_link_severed = true;
    assert!(
        mgr.is_last_real_action(sequence_id, 0),
        "Halt's nulled following pointer must hide physically adjacent dead elements"
    );
}

#[test]
fn clearing_actor_goal_snapshots_removes_typed_and_generic_caches() {
    let owner = EntityId::Civilian(crate::entity_id::CivilianId(1));
    let mut mgr = SequenceManager::new();
    let mut turn = SequenceElement::new_generic(1, Command::Turn, Some(owner));
    turn.set_property(
        Field::RetainedMovementGoal,
        FieldValue::GeoPoint2D { x: 12.0, y: 34.0 },
    );
    let turn_id = mgr.launch_element(turn);
    let movement_id = mgr.launch_element(movement_elem(owner, OrderType::WalkingUpright));
    mgr.get_element_mut(movement_id, 0)
        .expect("movement exists")
        .retained_movement_goal = Some(crate::coordinates::MapPoint::new(56.0, 78.0));

    mgr.clear_retained_movement_goals_for_actor(owner);

    assert!(
        mgr.get_element(turn_id, 0)
            .expect("Turn exists")
            .get_property(Field::RetainedMovementGoal)
            .is_none(),
        "deferred FaceTo must not restore a movement goal cleared by the outgoing card"
    );
    assert!(
        mgr.get_element(movement_id, 0)
            .expect("movement exists")
            .retained_movement_goal
            .is_none(),
        "typed replacement movement snapshots must still be cleared"
    );
}

#[test]
fn non_interruptable_impossible_guard_only_protects_in_progress_owner() {
    let owner = EntityId::Pc(crate::entity_id::PcId(1));
    let mut mgr = SequenceManager::new();

    let mut todo = SequenceElement::new(1, Command::LeaveListen, Some(owner));
    todo.priority = SequencePriority::NonInterruptable;
    let todo_seq = mgr.launch_element(todo);
    mgr.element_impossible(todo_seq, 0);
    assert_eq!(
        mgr.get_element(todo_seq, 0).unwrap().state,
        SequenceState::Impossible,
        "preflight failure must reject a Todo non-interruptable element"
    );

    let mut active = SequenceElement::new(1, Command::EnterListen, Some(owner));
    active.priority = SequencePriority::NonInterruptable;
    let active_seq = mgr.launch_element(active);
    mgr.element_in_progress(active_seq, 0);
    mgr.element_impossible(active_seq, 0);
    assert_eq!(
        mgr.get_element(active_seq, 0).unwrap().state,
        SequenceState::InProgress,
        "an executing non-interruptable owner remains protected"
    );

    mgr.element_impossible_from_execute(active_seq, 0);
    assert_eq!(
        mgr.get_element(active_seq, 0).unwrap().state,
        SequenceState::Impossible,
        "the owner's intrinsic Execute abort follows Original release behavior"
    );
}

#[test]
fn death_cleanup_preserves_exact_dead_human_todo_whitelist() {
    let owner = EntityId::Pc(crate::entity_id::PcId(38));
    let mut mgr = SequenceManager::new();
    let admitted = [
        Command::ReceiveHitDamage,
        Command::ReceiveSwordDamage,
        Command::ReceiveArrowDamage,
        Command::ReceiveDamage,
        Command::ReceiveMobileDamage,
        Command::Wait,
        Command::GetKilledAtBottom,
    ]
    .map(|command| {
        let sequence = mgr.launch_element(SequenceElement::new(1, command, Some(owner)));
        (command, sequence)
    });
    let rejected = [Command::ReceiveStoneDamage, Command::WaitTimer].map(|command| {
        let sequence = mgr.launch_element(SequenceElement::new(1, command, Some(owner)));
        (command, sequence)
    });

    mgr.kill_owner_sequences(owner, SequenceId(u32::MAX));

    for (command, sequence) in admitted {
        assert_eq!(
            mgr.get_element(sequence, 0).unwrap().state,
            SequenceState::Todo,
            "dead Human::Instruct admits queued {command:?}"
        );
    }
    for (command, sequence) in rejected {
        assert_eq!(
            mgr.get_element(sequence, 0).unwrap().state,
            SequenceState::Interrupted,
            "dead Human::Instruct rejects queued {command:?}"
        );
    }
}

#[test]
fn death_cleanup_preserves_postponed_wait_transferred_to_damage_replacement() {
    let owner = EntityId::Soldier(crate::entity_id::SoldierId(197));
    let mut mgr = SequenceManager::new();

    let damage = mgr.launch_element(SequenceElement::new(1, Command::ReceiveDamage, Some(owner)));
    let wait = mgr.launch_element(SequenceElement::new(1, Command::Wait, Some(owner)));
    mgr.postpone_element(wait, 0);
    mgr.get_element_mut(damage, 0).unwrap().cross_postponed = Some((wait, 0));

    let rejected = mgr.launch_element(SequenceElement::new(1, Command::WaitTimer, Some(owner)));
    mgr.postpone_element(rejected, 0);

    mgr.kill_owner_sequences(owner, damage);

    assert_eq!(
        mgr.get_element(wait, 0).unwrap().state,
        SequenceState::Postponed,
        "Original Human::Kill leaves the dead-admissible Wait queued behind lethal damage"
    );
    assert_eq!(
        mgr.get_element(damage, 0).unwrap().cross_postponed,
        Some((wait, 0)),
        "death cleanup must preserve the replacement's transferred postponed chain"
    );
    assert_eq!(
        mgr.get_element(rejected, 0).unwrap().state,
        SequenceState::Interrupted,
        "non-whitelisted postponed work must still be discarded on death"
    );
}

fn pending_drop_ale_seek(
    owner: EntityId,
    destination: crate::coordinates::MapPoint,
    fallback_sector: crate::position_interface::SectorHandle,
) -> SequenceElement {
    let mut seek = SequenceElement::new_movement(
        1,
        Command::Seek,
        Some(owner),
        crate::order::OrderType::WalkingUpright,
    );
    let SequenceElementData::Movement {
        destination: seek_destination,
        layer,
        sector,
        flags,
        post_seek_sequence,
        ..
    } = &mut seek.data
    else {
        unreachable!()
    };
    *seek_destination = destination;
    *layer = 2;
    *sector = Some(fallback_sector);
    *flags |= MoveFlags::SEEK;
    let mut post_seek = Sequence::new();
    post_seek.append_element(SequenceElement::new(1, Command::DropAle, Some(owner)));
    *post_seek_sequence = Some(post_seek.into_post_seek());
    seek
}

fn recorded_drop_ale_failure() -> crate::gate::RecordedGatePath {
    crate::gate::RecordedGatePath {
        source_sector: crate::sector::SectorNumber::new(133),
        source_sector_index: crate::fast_find_grid::SectorIndex::new(57),
        source_layer: 11,
        outcome: crate::gate::RecordedGateOutcome::Failure,
    }
}

#[test]
fn delayed_drop_ale_route_overwrites_fallback_only_while_seek_is_pending() {
    let owner = EntityId::Pc(crate::entity_id::PcId(36));
    let destination = crate::coordinates::MapPoint::new(778.0, 1714.0);
    let fallback_sector = crate::position_interface::SectorHandle::new(25).unwrap();
    let authoritative_sector = crate::position_interface::SectorHandle::new(0).unwrap();
    let route = crate::gate::RecordedGatePath {
        source_sector: crate::sector::SectorNumber::new(133),
        source_sector_index: None,
        source_layer: 11,
        outcome: crate::gate::RecordedGateOutcome::Success(vec![crate::gate::GatePathStep {
            door_index: crate::gate::DoorIndex(7),
            direct: false,
        }]),
    };
    let mut manager = SequenceManager::new();

    assert!(
        !manager.inject_recorded_drop_ale_route(
            owner,
            destination,
            authoritative_sector,
            0,
            route.clone(),
        ),
        "an event arriving before its command must not mutate unrelated state"
    );
    let sequence =
        manager.launch_element(pending_drop_ale_seek(owner, destination, fallback_sector));
    let unrelated_destination = crate::coordinates::MapPoint::new(779.0, 1714.0);
    assert!(!manager.has_pending_drop_ale_route_candidate(owner, unrelated_destination));
    assert!(
        !manager.inject_recorded_drop_ale_route(
            owner,
            unrelated_destination,
            authoritative_sector,
            0,
            route.clone(),
        ),
        "a same-actor route with different goal bits must remain unrelated"
    );
    assert!(manager.inject_recorded_drop_ale_route(
        owner,
        destination,
        authoritative_sector,
        0,
        route.clone(),
    ));
    let element = manager.get_element(sequence, 0).unwrap();
    let SequenceElementData::Movement { sector, layer, .. } = &element.data else {
        unreachable!()
    };
    assert_eq!(*sector, Some(authoritative_sector));
    assert_eq!(*layer, 0);
    assert_eq!(element.recorded_gate_path, Some(route));

    manager.get_element_mut(sequence, 0).unwrap().state = SequenceState::Terminated;
    assert!(
        !manager.inject_recorded_drop_ale_route(
            owner,
            destination,
            fallback_sector,
            2,
            recorded_drop_ale_failure(),
        ),
        "terminal DropAle elements must not receive later route events"
    );
}

#[test]
#[should_panic(expected = "matched 2 pending point Seeks")]
fn delayed_drop_ale_route_rejects_multiple_pending_matches() {
    let owner = EntityId::Pc(crate::entity_id::PcId(36));
    let destination = crate::coordinates::MapPoint::new(778.0, 1714.0);
    let fallback_sector = crate::position_interface::SectorHandle::new(25).unwrap();
    let mut manager = SequenceManager::new();
    manager.launch_element(pending_drop_ale_seek(owner, destination, fallback_sector));
    manager.launch_element(pending_drop_ale_seek(owner, destination, fallback_sector));

    manager.inject_recorded_drop_ale_route(
        owner,
        destination,
        crate::position_interface::SectorHandle::new(0).unwrap(),
        0,
        recorded_drop_ale_failure(),
    );
}

#[test]
#[should_panic(expected = "already has a recorded gate route")]
fn delayed_drop_ale_route_rejects_a_second_exact_route_event() {
    let owner = EntityId::Pc(crate::entity_id::PcId(36));
    let destination = crate::coordinates::MapPoint::new(778.0, 1714.0);
    let fallback_sector = crate::position_interface::SectorHandle::new(25).unwrap();
    let mut manager = SequenceManager::new();
    manager.launch_element(pending_drop_ale_seek(owner, destination, fallback_sector));
    assert!(manager.inject_recorded_drop_ale_route(
        owner,
        destination,
        crate::position_interface::SectorHandle::new(0).unwrap(),
        0,
        recorded_drop_ale_failure(),
    ));

    manager.has_pending_drop_ale_route_candidate(owner, destination);
}

#[test]
fn recorded_drop_ale_route_survives_binary_metadata_and_full_json_roundtrips() {
    let owner = EntityId::Pc(crate::entity_id::PcId(36));
    let destination = crate::coordinates::MapPoint::new(778.0, 1714.0);
    let mut manager = SequenceManager::new();
    let sequence = manager.launch_element(pending_drop_ale_seek(
        owner,
        destination,
        crate::position_interface::SectorHandle::new(25).unwrap(),
    ));
    let route = recorded_drop_ale_failure();
    assert!(manager.inject_recorded_drop_ale_route(
        owner,
        destination,
        crate::position_interface::SectorHandle::new(0).unwrap(),
        0,
        route.clone(),
    ));
    let encoded = bitcode::encode(&route);
    let decoded_route: crate::gate::RecordedGatePath =
        bitcode::decode(&encoded).expect("decode recorded route metadata");
    assert_eq!(decoded_route, route);

    let element_json = serde_json::to_string(manager.get_element(sequence, 0).unwrap())
        .expect("encode complete pending DropAle element");
    let decoded_element: SequenceElement =
        serde_json::from_str(&element_json).expect("decode complete pending DropAle element");
    assert_eq!(decoded_element.recorded_gate_path, Some(route));
    let SequenceElementData::Movement {
        post_seek_sequence, ..
    } = decoded_element.data
    else {
        panic!("decoded element is not movement")
    };
    assert_eq!(
        post_seek_sequence.unwrap().elements[0].command,
        Command::DropAle
    );
}

#[test]
fn post_seek_sequence_is_one_level_and_native_bitcode_roundtrips() {
    let owner = EntityId::Pc(crate::entity_id::PcId(1));
    let mut continuation = Sequence::new();
    continuation.append_element(SequenceElement::new(1, Command::CrouchDown, Some(owner)));

    let mut seek = SequenceElement::new_movement(
        1,
        Command::Seek,
        Some(owner),
        crate::order::OrderType::WalkingUpright,
    );
    let SequenceElementData::Movement {
        post_seek_sequence, ..
    } = &mut seek.data
    else {
        unreachable!()
    };
    *post_seek_sequence = Some(continuation.into_post_seek());

    let mut root = Sequence::new();
    root.append_element(seek);
    let bytes = bitcode::encode(&root);
    let decoded: Sequence = bitcode::decode(&bytes).expect("decode one-level post-seek sequence");
    let SequenceElementData::Movement {
        post_seek_sequence: Some(decoded_continuation),
        ..
    } = &decoded.elements[0].data
    else {
        panic!("decoded Seek lost its continuation")
    };
    assert_eq!(
        decoded_continuation.elements[0].command,
        Command::CrouchDown
    );

    assert!(matches!(
        decoded.try_into_post_seek(),
        Err(SequenceInvariantError::NestedPostSeekSequence)
    ));
}
