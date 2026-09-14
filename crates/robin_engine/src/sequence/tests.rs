use super::*;
use crate::engine::{EngineInner, LevelAssets};
use crate::sim_rng::test_context;

fn live_sequence_fixture() -> (EngineInner, LevelAssets, EntityId) {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(
        crate::engine::test_support::actors::TestActor::pc(crate::element::Posture::Upright)
            .build(),
    );
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    (engine, assets, owner)
}

#[test]
fn interruption_reads_following_link_after_owner_callback() {
    let (mut engine, assets, owner) = live_sequence_fixture();
    let mut sequence = Sequence::new();
    for _ in 0..3 {
        sequence.append_element(SequenceElement::new(1, Command::Generic, Some(owner)));
    }
    let id = engine.orders.sequence_manager.launch_sequence(sequence);
    EngineInner::with_condolation_callback(
        move |engine, card| {
            if card.elem_idx == 0 {
                engine
                    .orders
                    .sequence_manager
                    .get_element_mut(id, 0)
                    .unwrap()
                    .legacy_v48 = Some(loaded_v48_state(Some(SequenceElementRef::new(id, 2))));
            }
        },
        || {
            engine.element_interrupted(
                &test_context(),
                &assets,
                &mut Vec::new(),
                id,
                0,
                CascadeFlags::FOLLOWING,
            )
        },
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(id, 1)
            .unwrap()
            .state,
        SequenceState::Todo
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(id, 2)
            .unwrap()
            .state,
        SequenceState::Interrupted
    );
}

#[test]
fn termination_starts_postponed_link_selected_by_owner_callback() {
    let (mut engine, assets, owner) = live_sequence_fixture();
    let root = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::Generic, Some(owner)));
    let old = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::Generic, Some(owner)));
    let replacement = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::Generic, Some(owner)));
    engine.postpone_element(&test_context(), &assets, &mut Vec::new(), old, 0);
    engine.postpone_element(&test_context(), &assets, &mut Vec::new(), replacement, 0);
    engine
        .orders
        .sequence_manager
        .get_element_mut(root, 0)
        .unwrap()
        .cross_postponed = Some((old, 0));
    EngineInner::with_condolation_callback(
        move |engine, card| {
            if card.seq_id == root {
                let element = engine
                    .orders
                    .sequence_manager
                    .get_element_mut(root, 0)
                    .unwrap();
                assert_eq!(
                    element.cross_postponed,
                    Some((old, 0)),
                    "the callback sees the current postponed pointer"
                );
                element.cross_postponed = Some((replacement, 0));
            }
        },
        || engine.element_terminated(&test_context(), &assets, &mut Vec::new(), root, 0),
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(old, 0)
            .unwrap()
            .state,
        SequenceState::Postponed
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(replacement, 0)
            .unwrap()
            .state,
        SequenceState::Postponed
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .is_registered_to_go(replacement, 0)
    );
    assert!(!engine.orders.sequence_manager.is_registered_to_go(old, 0));
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(root, 0)
            .unwrap()
            .cross_postponed,
        None
    );
}

#[test]
fn next_level_cascade_reads_callback_replaced_cross_sequence_chain() {
    let (mut engine, assets, owner) = live_sequence_fixture();
    let root = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::Generic, Some(owner)));
    let mut continuation = Sequence::new();
    continuation.append_element(SequenceElement::new(1, Command::Generic, Some(owner)));
    continuation.append_element(SequenceElement::new(2, Command::Generic, Some(owner)));
    let tail = engine.orders.sequence_manager.launch_sequence(continuation);
    EngineInner::with_condolation_callback(
        move |engine, card| {
            if card.seq_id == root {
                engine
                    .orders
                    .sequence_manager
                    .get_element_mut(root, 0)
                    .unwrap()
                    .legacy_v48 = Some(loaded_v48_state(Some(SequenceElementRef::new(tail, 0))));
            }
        },
        || {
            engine.element_interrupted(
                &test_context(),
                &assets,
                &mut Vec::new(),
                root,
                0,
                CascadeFlags::NEXT_LEVEL,
            )
        },
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(tail, 0)
            .unwrap()
            .state,
        SequenceState::Todo
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(tail, 1)
            .unwrap()
            .state,
        SequenceState::Interrupted
    );
}

#[test]
fn stop_preserves_live_replacement_following_link_after_nested_callback() {
    let (mut engine, assets, owner) = live_sequence_fixture();
    let mut sequence = Sequence::new();
    for priority in [
        SequencePriority::NonInterruptable,
        SequencePriority::Normal,
        SequencePriority::NonInterruptable,
    ] {
        let mut element = SequenceElement::new(1, Command::Generic, Some(owner));
        element.priority = priority;
        sequence.append_element(element);
    }
    let id = engine.orders.sequence_manager.launch_sequence(sequence);
    engine
        .orders
        .sequence_manager
        .get_element_mut(id, 1)
        .unwrap()
        .legacy_v48 = Some(loaded_v48_state(None));
    EngineInner::with_condolation_callback(
        move |engine, card| {
            if card.elem_idx == 1 {
                engine
                    .orders
                    .sequence_manager
                    .get_element_mut(id, 0)
                    .unwrap()
                    .legacy_v48 = Some(loaded_v48_state(Some(SequenceElementRef::new(id, 2))));
            }
        },
        || {
            engine.stop_owner_current_from_root(
                &test_context(),
                &assets,
                &mut Vec::new(),
                owner,
                Some((id, 0)),
                SequencePriority::Normal,
                &|_, element| element.priority,
            )
        },
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(id, 1)
            .unwrap()
            .state,
        SequenceState::Interrupted
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(id, 2)
            .unwrap()
            .state,
        SequenceState::Todo
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_sequence(id)
            .unwrap()
            .live_following_ref(0),
        Some(SequenceElementRef::new(id, 2))
    );
}

#[test]
fn populated_actor_indexes_round_trip_through_json() {
    let (mut engine, mut assets, owner) = live_sequence_fixture();
    let other = engine.add_test_entity(
        crate::engine::test_support::actors::TestActor::pc(crate::element::Posture::Upright)
            .build(),
    );
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    for owner in [owner, other] {
        let sequence = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::Move, Some(owner)));
        engine.element_in_progress(&test_context(), &assets, &mut Vec::new(), sequence, 0);
        engine
            .orders
            .sequence_manager
            .actor_instructing
            .insert(owner, vec![(SequenceElementRef::new(sequence, 0), true)]);
    }
    assert_eq!(engine.orders.sequence_manager.actor_live.len(), 2);
    assert_eq!(engine.orders.sequence_manager.actor_in_progress.len(), 2);
    let json = serde_json::to_string(&engine.orders.sequence_manager)
        .expect("serialize populated actor indexes");
    let restored: SequenceManager = serde_json::from_str(&json).expect("restore actor indexes");
    assert_eq!(
        restored.actor_live,
        engine.orders.sequence_manager.actor_live
    );
    assert_eq!(
        restored.actor_in_progress,
        engine.orders.sequence_manager.actor_in_progress
    );
    assert_eq!(
        restored.actor_instructing,
        engine.orders.sequence_manager.actor_instructing
    );
    assert_eq!(
        robin_util::state_hash::compute(&restored),
        robin_util::state_hash::compute(&engine.orders.sequence_manager),
    );
}

fn make_simple_element(level: u16, cmd: Command, owner: Option<EntityId>) -> SequenceElement {
    SequenceElement::new(level, cmd, owner)
}

#[test]
fn replacement_interruption_marks_outgoing_card_unselected() {
    let (mut engine, assets, owner) = live_sequence_fixture();
    let outgoing = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::Move, Some(owner)));
    engine.element_in_progress(&test_context(), &assets, &mut Vec::new(), outgoing, 0);
    let cards = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = cards.clone();
    EngineInner::with_condolation_callback(
        move |_, card| {
            observed.borrow_mut().push((card.seq_id, card.was_selected));
        },
        || {
            engine.element_interrupted_after_replacement_selected(
                &test_context(),
                &assets,
                &mut Vec::new(),
                outgoing,
                0,
                CascadeFlags::NEXT_LEVEL,
            );
        },
    );
    assert_eq!(*cards.borrow(), [(outgoing, false)]);
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

#[test]
fn postponed_shoulder_climb_resumes_only_for_its_completed_helper() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();
    let fixture_owner_1 = engine.add_test_entity(
        crate::engine::test_support::actors::TestActor::pc(crate::element::Posture::Upright)
            .build(),
    );
    let fixture_owner_2 = engine.add_test_entity(
        crate::engine::test_support::actors::TestActor::pc(crate::element::Posture::Upright)
            .build(),
    );
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);

    let climber = fixture_owner_0;
    let helper = fixture_owner_1;
    let other_helper = fixture_owner_2;

    let sequence = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new_interaction(
            2,
            Command::ClimbUpOnShoulders,
            Some(climber),
            Some(helper),
        ));

    assert!(matches!(
        engine.orders.sequence_manager.pop_next_hourglass_action(),
        Some(SequenceAction::InstructOwner {
            owner,
            sequence_id,
            element_index: 0,
        }) if owner == climber && sequence_id == sequence
    ));
    engine.postpone_element(&sim, &assets, &mut Vec::new(), sequence, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::Postponed
    );
    assert!(engine.orders.sequence_manager.hourglass().is_empty());

    engine
        .orders
        .sequence_manager
        .resume_postponed_climbs_for_helper(other_helper);
    assert!(engine.orders.sequence_manager.hourglass().is_empty());

    engine
        .orders
        .sequence_manager
        .resume_postponed_climbs_for_helper(helper);
    let resumed = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap();
    assert_eq!(resumed.state, SequenceState::Todo);
    assert_eq!(resumed.posture_after_transition, Posture::Undefined);
    assert!(matches!(
        engine.orders.sequence_manager.pop_next_hourglass_action(),
        Some(SequenceAction::InstructOwner {
            owner,
            sequence_id,
            element_index: 0,
        }) if owner == climber && sequence_id == sequence
    ));
}

/// The `RecordPlayAnim*` natives at natives/mod.rs:2680-2729 write
/// `Field::AnimationId` as `FieldValue::Animation(OrderType)` on a
/// generic sequence element.  The `Command::PlayAnim*` dispatch in
/// `tick.rs` reads it back out via `get_property` and destructures
/// the same variant to feed `force_animation`.  Verify the
/// round-trip end-to-end.
#[test]
fn lazy_stop_priority_updates_cached_summary_like_a_rebuild() {
    for resolved in [
        SequencePriority::None,
        SequencePriority::Script,
        SequencePriority::NonInterruptable,
    ] {
        let (mut engine, assets, owner) = live_sequence_fixture();
        let root = engine
            .orders
            .sequence_manager
            .launch_element(movement_elem(owner, OrderType::WalkingUpright));
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .actor_stop_summary(owner)
                .unwrap()
                .weakest_priority,
            SequencePriority::NotYetSet
        );
        let expected = if resolved == SequencePriority::None {
            SequencePriority::Normal
        } else {
            resolved
        };
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .resolve_element_stop_priority(root, 0, &|_| resolved),
            expected
        );
        let (mut rebuilt, _, _) = live_sequence_fixture();
        rebuilt.orders.sequence_manager = engine.orders.sequence_manager.clone();
        rebuilt.orders.sequence_manager.rebuild_indices();
        let cached = engine
            .orders
            .sequence_manager
            .actor_stop_summary(owner)
            .unwrap();
        let fresh = rebuilt
            .orders
            .sequence_manager
            .actor_stop_summary(owner)
            .unwrap();
        assert_eq!(cached.weakest_priority, expected);
        assert_eq!(cached.weakest_priority, fresh.weakest_priority);
        assert_eq!(cached.cross_only, fresh.cross_only);
        for candidate in [&mut engine, &mut rebuilt] {
            candidate.stop_owner_current_from_root(
                &test_context(),
                &assets,
                &mut Vec::new(),
                owner,
                Some((root, 0)),
                SequencePriority::Preference,
                &|_, _| panic!("priority was already resolved"),
            );
        }
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(root, 0)
                .unwrap()
                .state,
            rebuilt
                .orders
                .sequence_manager
                .get_element(root, 0)
                .unwrap()
                .state
        );
        assert_eq!(
            robin_util::state_hash::compute(&engine.orders.sequence_manager),
            robin_util::state_hash::compute(&rebuilt.orders.sequence_manager)
        );
    }
}

#[test]
fn lazy_stop_priority_preserves_already_resolved_none_and_other_priorities() {
    let mut manager = SequenceManager::new();
    for priority in [
        SequencePriority::None,
        SequencePriority::Normal,
        SequencePriority::Script,
    ] {
        let mut element = SequenceElement::new(1, Command::Wait, None);
        element.priority = priority;
        let sequence = manager.launch_element(element);
        assert_eq!(
            manager.resolve_element_stop_priority(sequence, 0, &|_| panic!(
                "resolved priorities must not call the resolver"
            )),
            priority
        );
    }
}

#[test]
fn movement_stop_priority_refreshes_live_actor_summary_even_when_too_strong_to_rewrite() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;

    let mut movement = movement_elem(owner, OrderType::WalkingUpright);
    movement.push_order(Order::test_new(OrderType::WalkingUpright, 100.0, 0.0));
    let sequence = engine.orders.sequence_manager.launch_element(movement);
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), sequence, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .actor_stop_summary(owner)
            .unwrap()
            .weakest_priority,
        SequencePriority::NotYetSet
    );
    assert!(!engine.stop_movement_from_root(
        &sim,
        &assets,
        &mut Vec::new(),
        owner,
        (sequence, 0),
        crate::coordinates::MapPoint::new(0.0, 0.0),
        SequencePriority::Preference,
        &|_, _| SequencePriority::Script,
    ));
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .actor_stop_summary(owner)
            .unwrap()
            .weakest_priority,
        SequencePriority::Script
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .orders
            .front()
            .unwrap()
            .order_type,
        OrderType::WalkingUpright
    );
}

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

    assert_eq!(
        effects.cascade_after_card,
        Some((0, SequenceState::Interrupted, CascadeFlags::NEXT_LEVEL))
    );
    let id = seq.id;
    let mut manager = SequenceManager::new();
    manager.sequences.insert(id, seq);
    assert_eq!(
        manager.live_cascade_target(id, 0, CascadeFlags::NEXT_LEVEL),
        Some(SequenceElementRef::new(id, 1))
    );
    // A completion callback may sever the next link before the cascade resumes.
    manager
        .get_sequence_mut(id)
        .unwrap()
        .sever_following_link(0);
    assert_eq!(
        manager.live_cascade_target(id, 0, CascadeFlags::NEXT_LEVEL),
        None
    );
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

    let effects = seq.set_element_state(0, SequenceState::Interrupted, CascadeFlags::empty());

    assert_eq!(effects.start_postponed, None);
    assert_eq!(seq.elements[0].postponed_element_index, Some(1));
    assert_eq!(seq.elements[0].cross_postponed, None);
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

/// Original-game next-element sequence advancement
/// in the original game advances the cursor and
/// running count first, then walks that stable range in element order:
/// WAIT calls `Go()` inline, NORMAL is registered on the manager FIFO,
/// and an immediate command executes inside that registration. A WAIT
/// callback may terminate synchronously, enter `Ready()`, and dispatch a
/// WAIT successor before the outer launch/callback chain unwinds.
#[test]
fn wait_instruction_runs_before_immediate_sibling_and_leaves_normal_work_on_fifo() {
    let (mut engine, assets, owner) = live_sequence_fixture();
    let mut sequence = Sequence::new();
    let mut normal = make_simple_element(1, Command::Move, Some(owner));
    normal.priority = SequencePriority::Normal;
    sequence.append_element(normal);
    let mut wait = make_simple_element(1, Command::Wait, Some(owner));
    wait.priority = SequencePriority::Wait;
    sequence.append_element(wait);
    let mut immediate = make_simple_element(1, Command::LockUser, None);
    immediate.priority = SequencePriority::Normal;
    sequence.append_element(immediate);
    let id = engine.launch_sequence(sequence);
    engine
        .drain_script_synchronous_actions(&test_context(), &assets, &mut Vec::new())
        .unwrap();
    assert!(engine.players.user_locked);
    let manager = &engine.orders.sequence_manager;
    assert_eq!(
        manager.get_element(id, 1).unwrap().state,
        SequenceState::InProgress
    );
    assert!(!manager.get_element(id, 1).unwrap().orders.is_empty());
    assert_eq!(
        manager.get_element(id, 2).unwrap().state,
        SequenceState::Terminated
    );
    assert_eq!(
        manager.elements_to_go.iter().copied().collect::<Vec<_>>(),
        vec![(id, 0)]
    );
}

#[test]
fn wait_priority_instruction_bypasses_hourglass() {
    let (mut engine, assets, owner) = live_sequence_fixture();
    let mut wait = make_simple_element(1, Command::Wait, Some(owner));
    wait.priority = SequencePriority::Wait;
    let id = engine.orders.sequence_manager.launch_element(wait);
    engine
        .drain_script_synchronous_actions(&test_context(), &assets, &mut Vec::new())
        .unwrap();
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(id, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .get_element(id, 0)
            .unwrap()
            .orders
            .is_empty()
    );
    assert!(engine.orders.sequence_manager.elements_to_go.is_empty());
}

#[test]
fn wait_completion_instructs_next_level_before_returning() {
    let (mut engine, assets, owner) = live_sequence_fixture();
    let mut sequence = Sequence::new();
    for level in [1, 2] {
        let mut wait = make_simple_element(level, Command::Wait, Some(owner));
        wait.priority = SequencePriority::Wait;
        sequence.append_element(wait);
    }
    let id = engine.launch_sequence(sequence);
    engine
        .drain_script_synchronous_actions(&test_context(), &assets, &mut Vec::new())
        .unwrap();
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(id, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );
    engine.element_terminated(&test_context(), &assets, &mut Vec::new(), id, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(id, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(id, 1)
            .unwrap()
            .state,
        SequenceState::InProgress
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .get_element(id, 1)
            .unwrap()
            .orders
            .is_empty()
    );
    assert!(engine.orders.sequence_manager.elements_to_go.is_empty());
}

#[test]
fn manager_element_terminated_advances() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(1, Command::Move, Some(fixture_owner_0)));
    seq.append_element(make_simple_element(2, Command::Turn, Some(fixture_owner_0)));

    let seq_id = engine.orders.sequence_manager.launch_sequence(seq);

    // Drain the first hourglass
    let _ = engine.orders.sequence_manager.hourglass();

    // Mark element 0 as in-progress then terminated
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), seq_id, 0);
    engine.element_terminated(&sim, &assets, &mut Vec::new(), seq_id, 0);

    // The next level's element should now be queued
    let actions = engine.orders.sequence_manager.hourglass();
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        SequenceAction::InstructOwner { element_index, .. } => assert_eq!(*element_index, 1),
        other => panic!("expected InstructOwner for element 1, got {:?}", other),
    }
}

#[test]
fn live_hourglass_places_normal_successor_after_older_fifo_work() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();
    let fixture_owner_1 = engine.add_test_entity(
        crate::engine::test_support::actors::TestActor::pc(crate::element::Posture::Upright)
            .build(),
    );
    let fixture_owner_2 = engine.add_test_entity(
        crate::engine::test_support::actors::TestActor::pc(crate::element::Posture::Upright)
            .build(),
    );
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);

    let owner = fixture_owner_0;

    let mut route = Sequence::new();
    route.append_element(make_simple_element(1, Command::AssertPosition, Some(owner)));
    route.append_element(make_simple_element(2, Command::Move, Some(owner)));
    let route_id = engine.orders.sequence_manager.launch_sequence(route);

    let older_owner_a = fixture_owner_1;
    let older_owner_b = fixture_owner_2;
    let older_a = engine
        .orders
        .sequence_manager
        .launch_element(make_simple_element(
            1,
            Command::LookLeft,
            Some(older_owner_a),
        ));
    let older_b = engine
        .orders
        .sequence_manager
        .launch_element(make_simple_element(
            1,
            Command::LookRight,
            Some(older_owner_b),
        ));

    assert!(matches!(
        engine.orders.sequence_manager.pop_next_hourglass_action(),
        Some(SequenceAction::InstructOwner {
            sequence_id,
            element_index: 0,
            ..
        }) if sequence_id == route_id
    ));

    // AssertPosition terminates inside actor translation. Ready registers
    // the level-2 Move at the live manager FIFO tail before Go returns.
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), route_id, 0);
    engine.element_terminated(&sim, &assets, &mut Vec::new(), route_id, 0);

    let remaining = [
        engine.orders.sequence_manager.pop_next_hourglass_action(),
        engine.orders.sequence_manager.pop_next_hourglass_action(),
        engine.orders.sequence_manager.pop_next_hourglass_action(),
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
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;

    let mut old = Sequence::new();
    old.append_element(make_simple_element(1, Command::PassDoor, Some(owner)));
    old.append_element(make_simple_element(2, Command::Move, Some(owner)));
    let old_id = engine.orders.sequence_manager.launch_sequence(old);
    let _ = engine.orders.sequence_manager.hourglass();
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), old_id, 0);

    let replacement_id = engine
        .orders
        .sequence_manager
        .launch_element(make_simple_element(1, Command::AssertPosition, Some(owner)));
    let _ = engine.orders.sequence_manager.hourglass();
    engine.postpone_element(&sim, &assets, &mut Vec::new(), replacement_id, 0);
    engine
        .orders
        .sequence_manager
        .get_element_mut(old_id, 0)
        .unwrap()
        .cross_postponed = Some((replacement_id, 0));

    engine.element_terminated(&sim, &assets, &mut Vec::new(), old_id, 0);

    let actions: Vec<_> =
        std::iter::from_fn(|| engine.orders.sequence_manager.pop_next_hourglass_action()).collect();
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
fn released_cross_postponed_assertions_keep_ready_before_postponed_fifo() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;

    let mut old = Sequence::new();
    old.append_element(make_simple_element(4, Command::PassDoor, Some(owner)));
    old.append_element(make_simple_element(5, Command::AssertPosition, Some(owner)));
    old.append_element(make_simple_element(6, Command::Move, Some(owner)));
    let old_id = engine.orders.sequence_manager.launch_sequence(old);
    let _ = engine.orders.sequence_manager.hourglass();
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), old_id, 0);

    let mut replacement = Sequence::new();
    replacement.append_element(make_simple_element(1, Command::AssertPosition, Some(owner)));
    replacement.append_element(make_simple_element(2, Command::Move, Some(owner)));
    let replacement_id = engine.orders.sequence_manager.launch_sequence(replacement);
    let _ = engine.orders.sequence_manager.hourglass();
    engine.postpone_element(&sim, &assets, &mut Vec::new(), replacement_id, 0);
    engine
        .orders
        .sequence_manager
        .get_element_mut(old_id, 0)
        .unwrap()
        .cross_postponed = Some((replacement_id, 0));

    engine.element_terminated(&sim, &assets, &mut Vec::new(), old_id, 0);

    // Ready registers the current sequence's assertion before postponed
    // startup registers the replacement assertion. Completing either appends
    // its following Move after the other work already in the manager FIFO.
    assert!(matches!(
        engine.orders.sequence_manager.pop_next_hourglass_action(),
        Some(SequenceAction::InstructOwner {
            sequence_id,
            element_index: 1,
            ..
        }) if sequence_id == old_id
    ));
    engine.element_terminated(&sim, &assets, &mut Vec::new(), old_id, 1);

    assert!(matches!(
        engine.orders.sequence_manager.pop_next_hourglass_action(),
        Some(SequenceAction::InstructOwner {
            sequence_id,
            element_index: 0,
            ..
        }) if sequence_id == replacement_id
    ));
    engine.element_terminated(&sim, &assets, &mut Vec::new(), replacement_id, 0);

    assert!(matches!(
        engine.orders.sequence_manager.pop_next_hourglass_action(),
        Some(SequenceAction::InstructOwner {
            sequence_id,
            element_index: 2,
            ..
        }) if sequence_id == old_id
    ));
    assert!(matches!(
        engine.orders.sequence_manager.pop_next_hourglass_action(),
        Some(SequenceAction::InstructOwner {
            sequence_id,
            element_index: 1,
            ..
        }) if sequence_id == replacement_id
    ));
}

#[test]
fn released_multi_door_route_keeps_ready_before_postponed_fifo() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;

    let mut old = Sequence::new();
    old.append_element(make_simple_element(4, Command::PassDoor, Some(owner)));
    old.append_element(make_simple_element(5, Command::AssertPosition, Some(owner)));
    old.append_element(make_simple_element(6, Command::Move, Some(owner)));
    let old_id = engine.orders.sequence_manager.launch_sequence(old);
    let _ = engine.orders.sequence_manager.hourglass();
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), old_id, 0);

    let mut replacement = Sequence::new();
    replacement.append_element(make_simple_element(1, Command::AssertPosition, Some(owner)));
    replacement.append_element(make_simple_element(2, Command::Move, Some(owner)));
    replacement.append_element(make_simple_element(3, Command::AssertPosition, Some(owner)));
    replacement.append_element(make_simple_element(4, Command::PassDoor, Some(owner)));
    let replacement_id = engine.orders.sequence_manager.launch_sequence(replacement);
    let _ = engine.orders.sequence_manager.hourglass();
    engine.postpone_element(&sim, &assets, &mut Vec::new(), replacement_id, 0);
    engine
        .orders
        .sequence_manager
        .get_element_mut(old_id, 0)
        .unwrap()
        .cross_postponed = Some((replacement_id, 0));

    engine.element_terminated(&sim, &assets, &mut Vec::new(), old_id, 0);

    assert!(matches!(
        engine.orders.sequence_manager.pop_next_hourglass_action(),
        Some(SequenceAction::InstructOwner {
            sequence_id,
            element_index: 1,
            ..
        }) if sequence_id == old_id
    ));
    assert!(matches!(
        engine.orders.sequence_manager.pop_next_hourglass_action(),
        Some(SequenceAction::InstructOwner {
            sequence_id,
            element_index: 0,
            ..
        }) if sequence_id == replacement_id
    ));
}

#[test]
fn released_same_sequence_postponed_action_clears_blocker_edge() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;

    let mut sequence = Sequence::new();
    sequence.append_element(make_simple_element(1, Command::Wait, Some(owner)));
    sequence.append_element(make_simple_element(1, Command::AssertPosition, Some(owner)));
    let sequence_id = engine.orders.sequence_manager.launch_sequence(sequence);
    let _ = engine.orders.sequence_manager.hourglass();

    engine.element_in_progress(&sim, &assets, &mut Vec::new(), sequence_id, 0);
    engine.postpone_element(&sim, &assets, &mut Vec::new(), sequence_id, 1);
    engine
        .orders
        .sequence_manager
        .get_element_mut(sequence_id, 0)
        .unwrap()
        .postponed_element_index = Some(1);

    engine.element_terminated(&sim, &assets, &mut Vec::new(), sequence_id, 0);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .unwrap()
            .postponed_element_index,
        None,
        "starting a postponed sequence element must detach the released edge"
    );
    assert!(matches!(
        engine.orders.sequence_manager.hourglass().as_slice(),
        [SequenceAction::InstructOwner {
            owner: action_owner,
            sequence_id: action_sequence,
            element_index: 1,
        }] if *action_owner == owner && *action_sequence == sequence_id
    ));
}

#[test]
fn finishing_condolation_stops_at_nested_card_before_cascade_continues() {
    let (mut engine, assets, owner) = live_sequence_fixture();
    let mut sequence = Sequence::new();
    for level in 1..=3 {
        sequence.append_element(SequenceElement::new(level, Command::Generic, Some(owner)));
    }
    let sequence_id = engine.orders.sequence_manager.launch_sequence(sequence);
    let cards = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = cards.clone();
    EngineInner::with_condolation_callback(
        move |engine, card| {
            observed.borrow_mut().push(card.elem_idx);
            if card.elem_idx < 2 {
                assert_eq!(
                    engine
                        .orders
                        .sequence_manager
                        .get_element(sequence_id, usize::from(card.elem_idx) + 1)
                        .unwrap()
                        .state,
                    SequenceState::Todo,
                    "the next element is untouched during its predecessor's callback"
                );
            }
        },
        || {
            engine.element_interrupted(
                &test_context(),
                &assets,
                &mut Vec::new(),
                sequence_id,
                0,
                CascadeFlags::NEXT_LEVEL,
            )
        },
    );
    assert_eq!(*cards.borrow(), [0, 1, 2]);
    assert!((0..3).all(|index| {
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, index)
            .unwrap()
            .state
            == SequenceState::Interrupted
    }));
}

#[test]
fn stop_owner_interrupts_actor_work_postponed_by_injury() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;

    // A preference action is current until an injury postpones it.
    let mut parry = make_simple_element(1, Command::ParrySword, Some(owner));
    parry.priority = SequencePriority::Preference;
    let parry_seq = engine.orders.sequence_manager.launch_element(parry);
    let _ = engine.orders.sequence_manager.hourglass();
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), parry_seq, 0);
    engine.postpone_element(&sim, &assets, &mut Vec::new(), parry_seq, 0);

    // During the injury's terminal condolence callback, actor stopping sees
    // the injury as current in the original and recursively stops the
    // postponed parry. Model the cross-sequence postponed link explicitly.
    let mut injury = make_simple_element(1, Command::ReceiveSwordDamage, Some(owner));
    injury.priority = SequencePriority::Injury;
    let injury_seq = engine.orders.sequence_manager.launch_element(injury);
    let _ = engine.orders.sequence_manager.hourglass();
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), injury_seq, 0);
    engine
        .orders
        .sequence_manager
        .get_element_mut(injury_seq, 0)
        .unwrap()
        .cross_postponed = Some((parry_seq, 0));

    engine.stop_owner(
        &sim,
        &assets,
        &mut Vec::new(),
        owner,
        SequencePriority::Preference,
        &|_, elem| elem.priority,
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(injury_seq, 0)
            .unwrap()
            .state,
        SequenceState::InProgress,
        "Preference StopAll must not interrupt the stronger injury callback"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(parry_seq, 0)
            .unwrap()
            .state,
        SequenceState::Interrupted,
        "the parry hidden underneath the injury must not resume after StopAll"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(injury_seq, 0)
            .unwrap()
            .cross_postponed,
        None,
        "the injury must not retain a resumable link to stopped actor work"
    );
}

#[test]
fn split_stop_scans_work_registered_by_selected_element_callback() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;
    let resolver = |_: &EngineInner, element: &SequenceElement| element.priority;

    let mut current = make_simple_element(1, Command::Turn, Some(owner));
    current.priority = SequencePriority::Normal;
    let current_seq = engine.orders.sequence_manager.launch_element(current);
    let _ = engine.orders.sequence_manager.hourglass();
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), current_seq, 0);

    engine.stop_owner_current_from_root(
        &sim,
        &assets,
        &mut Vec::new(),
        owner,
        Some((current_seq, 0)),
        SequencePriority::Preference,
        &resolver,
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(current_seq, 0)
            .unwrap()
            .state,
        SequenceState::Interrupted
    );

    // Model a completion callback triggering AI decisions that register overview work while
    // Actor stopping is still between its selected and pending phases.
    let mut callback_look = make_simple_element(1, Command::LookLeft, Some(owner));
    callback_look.priority = SequencePriority::Normal;
    let callback_look_seq = engine.orders.sequence_manager.launch_element(callback_look);

    let pending_snapshot = engine
        .orders
        .sequence_manager
        .pending_elements_for_owner(owner);

    // Model a card from the pending scan registering another command
    // after the scan captured its stable membership.
    let mut pending_card_look = make_simple_element(1, Command::LookRight, Some(owner));
    pending_card_look.priority = SequencePriority::Normal;
    let pending_card_look_seq = engine
        .orders
        .sequence_manager
        .launch_element(pending_card_look);
    for root in pending_snapshot {
        engine.stop_pending_element_from_root(
            &sim,
            &assets,
            &mut Vec::new(),
            owner,
            root,
            SequencePriority::Preference,
            &resolver,
        );
    }
    engine
        .orders
        .sequence_manager
        .compact_terminal_elements_to_go();
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(callback_look_seq, 0)
            .unwrap()
            .state,
        SequenceState::Interrupted,
        "pending-work cancellation must see work registered by the selected element's synchronous callback"
    );

    // Snapshot the list size before stopping elements that have not launched.
    // Work registered by a captured entry's card belongs to the next
    // manager update, not this scan.
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(pending_card_look_seq, 0)
            .unwrap()
            .state,
        SequenceState::Todo
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .v48_elements_to_go()
            .contains(&(pending_card_look_seq, 0))
    );
}

#[test]
fn stop_owner_batches_cleanup_for_long_cross_postponed_chain() {
    // Match the native game thread's stack budget for this deep recursive graph.
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
            let sim = test_context();
            let fixture_owner_1 = engine.add_test_entity(
                crate::engine::test_support::actors::TestActor::pc(
                    crate::element::Posture::Upright,
                )
                .build(),
            );
            crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);

            let owner = fixture_owner_0;
            let unrelated_owner = fixture_owner_1;

            // Replays retain a large amount of historical sequence state until the
            // Friday cleanup pass. Keep enough unrelated sequences here to catch a
            // regression that scans the whole manager for every stopped chain node.
            for _ in 0..4096 {
                engine
                    .orders
                    .sequence_manager
                    .launch_element(make_simple_element(1, Command::Wait, Some(unrelated_owner)));
            }

            let mut chain = Vec::with_capacity(4096);
            for _ in 0..4096 {
                let mut element = make_simple_element(1, Command::EnterSwordfight, Some(owner));
                element.priority = SequencePriority::Normal;
                let sequence = engine.orders.sequence_manager.launch_element(element);
                engine.postpone_element(&sim, &assets, &mut Vec::new(), sequence, 0);
                if let Some(&previous) = chain.last() {
                    engine
                        .orders
                        .sequence_manager
                        .get_element_mut(previous, 0)
                        .unwrap()
                        .cross_postponed = Some((sequence, 0));
                }
                chain.push(sequence);
            }

            engine.stop_owner_current_from_root(
                &sim,
                &assets,
                &mut Vec::new(),
                owner,
                Some((chain[0], 0)),
                SequencePriority::Preference,
                &|_, element| element.priority,
            );

            for sequence in chain {
                let element = engine
                    .orders
                    .sequence_manager
                    .get_element(sequence, 0)
                    .unwrap();
                assert_eq!(element.state, SequenceState::Interrupted);
                assert_eq!(element.cross_postponed, None);
            }
        })
        .expect("spawn deep sequence test")
        .join()
        .expect("deep sequence test failed");
}

#[test]
fn repeated_preference_stops_skip_growing_strong_postponed_prefix() {
    // Match the native game thread's stack budget for this deep recursive graph.
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
            let sim = test_context();

            let owner = fixture_owner_0;

            let mut root_element = make_simple_element(1, Command::QuitSwordfight, Some(owner));
            root_element.priority = SequencePriority::PostponeEverythingButInjuries;
            let root = engine.orders.sequence_manager.launch_element(root_element);
            engine.element_in_progress(&sim, &assets, &mut Vec::new(), root, 0);

            // EnterSwordfight can add one equal-priority postponed element between
            // successive preference-priority stops while preparing for swordfights. The original game's
            // pointer walk is effect-free for this graph; rescanning the full prefix
            // after every append makes the Rust representation triangular.
            let mut tail = root;
            let mut chain = Vec::with_capacity(8192);
            for _ in 0..8192 {
                let mut element = make_simple_element(1, Command::EnterSwordfight, Some(owner));
                element.priority = SequencePriority::PostponeEverythingButInjuries;
                let sequence = engine.orders.sequence_manager.launch_element(element);
                engine.postpone_element(&sim, &assets, &mut Vec::new(), sequence, 0);
                engine
                    .orders
                    .sequence_manager
                    .get_element_mut(tail, 0)
                    .unwrap()
                    .cross_postponed = Some((sequence, 0));
                tail = sequence;
                chain.push(sequence);

                engine.stop_owner_current_from_root(
                    &sim,
                    &assets,
                    &mut Vec::new(),
                    owner,
                    Some((root, 0)),
                    SequencePriority::Preference,
                    &|_, element| element.priority,
                );
            }

            assert_eq!(
                engine
                    .orders
                    .sequence_manager
                    .get_element(root, 0)
                    .unwrap()
                    .state,
                SequenceState::InProgress
            );
            assert!(chain.iter().all(|sequence| {
                engine
                    .orders
                    .sequence_manager
                    .get_element(*sequence, 0)
                    .unwrap()
                    .state
                    == SequenceState::Postponed
            }));

            // The ceiling is only an admission shortcut. A newly linked weak tail
            // must disable it and remain observable to the exact Original traversal.
            let mut weak = make_simple_element(1, Command::Turn, Some(owner));
            weak.priority = SequencePriority::Normal;
            let weak = engine.orders.sequence_manager.launch_element(weak);
            engine.postpone_element(&sim, &assets, &mut Vec::new(), weak, 0);
            engine
                .orders
                .sequence_manager
                .get_element_mut(tail, 0)
                .unwrap()
                .cross_postponed = Some((weak, 0));
            engine.stop_owner_current_from_root(
                &sim,
                &assets,
                &mut Vec::new(),
                owner,
                Some((root, 0)),
                SequencePriority::Preference,
                &|_, element| element.priority,
            );
            assert_eq!(
                engine
                    .orders
                    .sequence_manager
                    .get_element(weak, 0)
                    .unwrap()
                    .state,
                SequenceState::Interrupted
            );
        })
        .expect("spawn deep sequence test")
        .join()
        .expect("deep sequence test failed");
}

#[test]
fn strong_owner_summary_does_not_hide_weak_same_sequence_successor() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();
    let fixture_owner_1 = engine.add_test_entity(
        crate::engine::test_support::actors::TestActor::pc(crate::element::Posture::Upright)
            .build(),
    );
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);

    let owner = fixture_owner_0;
    let successor_owner = fixture_owner_1;
    let mut sequence = Sequence::new();
    let mut root = make_simple_element(1, Command::QuitSwordfight, Some(owner));
    root.priority = SequencePriority::PostponeEverythingButInjuries;
    sequence.append_element(root);
    let mut successor = make_simple_element(2, Command::Turn, Some(successor_owner));
    successor.priority = SequencePriority::Normal;
    sequence.append_element(successor);
    let sequence = engine.orders.sequence_manager.launch_sequence(sequence);

    engine.stop_owner_current_from_root(
        &sim,
        &assets,
        &mut Vec::new(),
        owner,
        Some((sequence, 0)),
        SequencePriority::Preference,
        &|_, element| element.priority,
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 1)
            .unwrap()
            .state,
        SequenceState::Interrupted,
        "actor-wide priority shortcut must preserve Original's same-sequence recursion"
    );
}

#[test]
fn repeated_selected_stops_do_not_scan_unrelated_retained_sequences() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();
    let fixture_owner_1 = engine.add_test_entity(
        crate::engine::test_support::actors::TestActor::pc(crate::element::Posture::Upright)
            .build(),
    );
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);

    let owner = fixture_owner_0;
    let unrelated_owner = fixture_owner_1;

    for _ in 0..4096 {
        engine
            .orders
            .sequence_manager
            .launch_element(make_simple_element(1, Command::Turn, Some(unrelated_owner)));
    }
    let _ = engine.orders.sequence_manager.hourglass();

    let mut roots = Vec::with_capacity(2048);
    for _ in 0..2048 {
        let mut element = make_simple_element(1, Command::EnterSwordfight, Some(owner));
        element.priority = SequencePriority::Normal;
        let sequence = engine.orders.sequence_manager.launch_element(element);
        engine.postpone_element(&sim, &assets, &mut Vec::new(), sequence, 0);
        roots.push(sequence);
    }

    for &sequence in &roots {
        engine.stop_owner_current_from_root(
            &sim,
            &assets,
            &mut Vec::new(),
            owner,
            Some((sequence, 0)),
            SequencePriority::Preference,
            &|_, element| element.priority,
        );
    }

    for sequence in roots {
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .state,
            SequenceState::Interrupted
        );
    }
}

#[test]
fn stop_pending_roots_do_not_scan_unrelated_retained_sequences() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();
    let fixture_owner_1 = engine.add_test_entity(
        crate::engine::test_support::actors::TestActor::pc(crate::element::Posture::Upright)
            .build(),
    );
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);

    let owner = fixture_owner_0;
    let unrelated_owner = fixture_owner_1;

    for _ in 0..4096 {
        engine
            .orders
            .sequence_manager
            .launch_element(make_simple_element(1, Command::Turn, Some(unrelated_owner)));
    }
    let _ = engine.orders.sequence_manager.hourglass();

    let mut roots = Vec::with_capacity(2048);
    for _ in 0..2048 {
        let mut element = make_simple_element(1, Command::EnterSwordfight, Some(owner));
        element.priority = SequencePriority::Normal;
        roots.push(engine.orders.sequence_manager.launch_element(element));
    }

    for &sequence in &roots {
        engine.stop_pending_element_from_root(
            &sim,
            &assets,
            &mut Vec::new(),
            owner,
            (sequence, 0),
            SequencePriority::Preference,
            &|_, element| element.priority,
        );
    }
    engine
        .orders
        .sequence_manager
        .compact_terminal_elements_to_go();

    for sequence in roots {
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .state,
            SequenceState::Interrupted
        );
    }
}

#[test]
fn stop_owner_walks_nested_cross_postponed_graph() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;

    let mut deepest = make_simple_element(1, Command::ParrySword, Some(owner));
    deepest.priority = SequencePriority::Preference;
    let deepest_seq = engine.orders.sequence_manager.launch_element(deepest);
    engine.postpone_element(&sim, &assets, &mut Vec::new(), deepest_seq, 0);

    let mut middle = make_simple_element(1, Command::EnterSwordfight, Some(owner));
    middle.priority = SequencePriority::Preference;
    let middle_seq = engine.orders.sequence_manager.launch_element(middle);
    engine.postpone_element(&sim, &assets, &mut Vec::new(), middle_seq, 0);
    engine
        .orders
        .sequence_manager
        .get_element_mut(middle_seq, 0)
        .unwrap()
        .cross_postponed = Some((deepest_seq, 0));

    let mut injury = make_simple_element(1, Command::ReceiveSwordDamage, Some(owner));
    injury.priority = SequencePriority::Injury;
    let injury_seq = engine.orders.sequence_manager.launch_element(injury);
    let _ = engine.orders.sequence_manager.hourglass();
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), injury_seq, 0);
    engine
        .orders
        .sequence_manager
        .get_element_mut(injury_seq, 0)
        .unwrap()
        .cross_postponed = Some((middle_seq, 0));

    engine.stop_owner(
        &sim,
        &assets,
        &mut Vec::new(),
        owner,
        SequencePriority::Preference,
        &|_, element| element.priority,
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(injury_seq, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );
    for sequence in [middle_seq, deepest_seq] {
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .state,
            SequenceState::Interrupted,
            "every recursively postponed actor action must be stopped"
        );
    }
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(injury_seq, 0)
            .unwrap()
            .cross_postponed,
        None
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(middle_seq, 0)
            .unwrap()
            .cross_postponed,
        None
    );
}

#[test]
fn stop_owner_walks_postponed_graph_from_pending_strong_blocker() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;

    let mut turn = make_simple_element(1, Command::Turn, Some(owner));
    turn.priority = SequencePriority::Normal;
    let turn_seq = engine.orders.sequence_manager.launch_element(turn);
    engine.postpone_element(&sim, &assets, &mut Vec::new(), turn_seq, 0);

    let mut attentive = make_simple_element(1, Command::EnterAttentiveMode, Some(owner));
    attentive.priority = SequencePriority::PostponeEverythingButInjuries;
    let attentive_seq = engine.orders.sequence_manager.launch_element(attentive);
    engine.postpone_element(&sim, &assets, &mut Vec::new(), attentive_seq, 0);
    engine
        .orders
        .sequence_manager
        .get_element_mut(attentive_seq, 0)
        .unwrap()
        .cross_postponed = Some((turn_seq, 0));

    let mut leave_attentive = make_simple_element(1, Command::LeaveAttentiveMode, Some(owner));
    leave_attentive.priority = SequencePriority::PostponeEverythingButInjuries;
    let leave_seq = engine
        .orders
        .sequence_manager
        .launch_element(leave_attentive);
    engine
        .orders
        .sequence_manager
        .get_element_mut(leave_seq, 0)
        .unwrap()
        .cross_postponed = Some((attentive_seq, 0));

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner),
        None,
        "Todo manager entries are not the actor's current element"
    );

    engine.stop_owner(
        &sim,
        &assets,
        &mut Vec::new(),
        owner,
        SequencePriority::Preference,
        &|_, element| element.priority,
    );

    for sequence in [leave_seq, attentive_seq] {
        assert_ne!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .state,
            SequenceState::Interrupted,
            "StopAll must preserve attentive-mode blockers"
        );
    }
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(turn_seq, 0)
            .unwrap()
            .state,
        SequenceState::Interrupted,
        "stopping follows a pending blocker's postponed reference even when the blocker is too strong to stop"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(attentive_seq, 0)
            .unwrap()
            .cross_postponed,
        None
    );
}

#[test]
fn stop_owner_does_not_scan_unselected_postponed_branches() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;

    // This branch belongs to the actor but is not reachable from the
    // actor's authoritative current element. Original-game actor stopping never
    // discovers it by scanning sequence ownership.
    let mut stale = make_simple_element(1, Command::EquipBow, Some(owner));
    stale.priority = SequencePriority::Preference;
    let stale_seq = engine.orders.sequence_manager.launch_element(stale);
    let _ = engine.orders.sequence_manager.hourglass();
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), stale_seq, 0);
    engine.postpone_element(&sim, &assets, &mut Vec::new(), stale_seq, 0);

    let mut current = make_simple_element(1, Command::UnequipBow, Some(owner));
    current.priority = SequencePriority::Preference;
    let current_seq = engine.orders.sequence_manager.launch_element(current);
    let _ = engine.orders.sequence_manager.hourglass();
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), current_seq, 0);

    engine.stop_owner(
        &sim,
        &assets,
        &mut Vec::new(),
        owner,
        SequencePriority::Preference,
        &|_, elem| elem.priority,
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(current_seq, 0)
            .unwrap()
            .state,
        SequenceState::Interrupted,
        "stopping an actor must stop its selected sequence element"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(stale_seq, 0)
            .unwrap()
            .state,
        SequenceState::Postponed,
        "unlinked postponed ownership is not an Original traversal root"
    );
}

#[test]
fn postpone_element_consumes_its_existing_manager_registration() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;
    let sequence_id = engine
        .orders
        .sequence_manager
        .launch_element(make_simple_element(1, Command::EquipBow, Some(owner)));

    engine.postpone_element(&sim, &assets, &mut Vec::new(), sequence_id, 0);

    assert!(
        engine.orders.sequence_manager.hourglass().is_empty(),
        "Postpone runs after Original's manager pop and must consume Rust's eager registration"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .unwrap()
            .state,
        SequenceState::Postponed
    );
}

#[test]
fn manager_friday_evening_cleanup() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(1, Command::Move, Some(fixture_owner_0)));
    let seq_id = engine.orders.sequence_manager.launch_sequence(seq);

    assert_eq!(engine.orders.sequence_manager.sequence_count(), 1);

    // Mark element as terminated
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), seq_id, 0);
    engine.element_terminated(&sim, &assets, &mut Vec::new(), seq_id, 0);

    // Now cleanup should remove it
    engine.orders.sequence_manager.friday_evening_cleanup();
    assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);
}

#[test]
fn manager_terminate_sequence() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(1, Command::Move, Some(fixture_owner_0)));
    seq.append_element(make_simple_element(2, Command::Turn, Some(fixture_owner_0)));
    let seq_id = engine.orders.sequence_manager.launch_sequence(seq);

    assert!(engine.terminate_sequence(&sim, &assets, &mut Vec::new(), seq_id));

    // Both elements should be interrupted
    let s = engine.orders.sequence_manager.get_sequence(seq_id).unwrap();
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

    // SendMessage with owner — owner branch.
    // Sequence completion invokes the owner callback during
    // registration rather than deferring the command to a later tick.
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
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();
    let fixture_owner_1 = engine.add_test_entity(
        crate::engine::test_support::actors::TestActor::pc(crate::element::Posture::Upright)
            .build(),
    );
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);

    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(1, Command::Move, Some(fixture_owner_0)));
    seq.append_element(make_simple_element(1, Command::Move, Some(fixture_owner_1)));
    seq.append_element(make_simple_element(2, Command::Turn, Some(fixture_owner_0)));

    let seq_id = engine.orders.sequence_manager.launch_sequence(seq);

    // Should get two actions (both level-1 elements)
    let actions = engine.orders.sequence_manager.hourglass();
    assert_eq!(actions.len(), 2);

    // Terminate both
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), seq_id, 0);
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), seq_id, 1);
    engine.element_terminated(&sim, &assets, &mut Vec::new(), seq_id, 0);

    // Level 2 not yet started — one still running
    let actions = engine.orders.sequence_manager.hourglass();
    assert!(actions.is_empty());

    engine.element_terminated(&sim, &assets, &mut Vec::new(), seq_id, 1);

    // Now level 2 should start
    let actions = engine.orders.sequence_manager.hourglass();
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
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;

    let mut current_seq = Sequence::new();
    current_seq.append_element(make_simple_element(1, Command::Move, Some(owner)));
    let current_seq_id = engine.orders.sequence_manager.launch_sequence(current_seq);
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), current_seq_id, 0);
    engine
        .orders
        .sequence_manager
        .elements_to_go
        .retain(|&(seq_id, elem_idx)| (seq_id, elem_idx) != (current_seq_id, 0));

    let mut postponed_seq = Sequence::new();
    postponed_seq.append_element(make_simple_element(
        1,
        Command::EnterSwordfight,
        Some(owner),
    ));
    let postponed_seq_id = engine
        .orders
        .sequence_manager
        .launch_sequence(postponed_seq);
    engine
        .orders
        .sequence_manager
        .elements_to_go
        .retain(|&(seq_id, elem_idx)| (seq_id, elem_idx) != (postponed_seq_id, 0));
    engine.postpone_element(&sim, &assets, &mut Vec::new(), postponed_seq_id, 0);

    // A postponed command elsewhere is not what Original's
    // current-element postponed pointer asks about.
    assert!(
        !engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched_or_postponed_by_current(
                owner,
                Command::EnterSwordfight
            )
    );

    engine
        .orders
        .sequence_manager
        .get_element_mut(current_seq_id, 0)
        .unwrap()
        .cross_postponed = Some((postponed_seq_id, 0));
    assert!(
        engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched_or_postponed_by_current(
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

    // The update removes the element from the launch list before
    // Go enters actor instruction handling. The actor selection remains live during
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
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let mut seq = Sequence::new();
    seq.append_element(make_simple_element(1, Command::Move, Some(fixture_owner_0)));
    seq.append_element(make_simple_element(1, Command::Turn, Some(fixture_owner_0)));
    let _seq_id = engine.orders.sequence_manager.launch_sequence(seq);

    engine.cancel_pending_move_commands(&sim, &assets, &mut Vec::new(), fixture_owner_0);

    // Only Turn should remain (Move was cancelled)
    let actions = engine.orders.sequence_manager.hourglass();
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
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;

    let mut selected = Sequence::new();
    selected.append_element(movement_elem(owner, OrderType::WalkingUpright));
    selected.append_element(movement_elem(owner, OrderType::WalkingUpright));
    let selected_id = engine.orders.sequence_manager.launch_sequence(selected);
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), selected_id, 0);

    let mut unrelated = Sequence::new();
    unrelated.append_element(movement_elem(owner, OrderType::WalkingUpright));
    let unrelated_id = engine.orders.sequence_manager.launch_sequence(unrelated);

    engine.orders.sequence_manager.make_fast(owner);

    for idx in 0..2 {
        let SequenceElementData::Movement { action, flags, .. } = &engine
            .orders
            .sequence_manager
            .get_element(selected_id, idx)
            .expect("selected chain element remains present")
            .data
        else {
            panic!("movement variant");
        };
        assert_eq!(*action, OrderType::RunningUpright);
        assert!(flags.contains(MoveFlags::FAST));
    }

    let SequenceElementData::Movement { action, flags, .. } = &engine
        .orders
        .sequence_manager
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
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;

    let mut sequence = Sequence::new();
    sequence.append_element(movement_elem(owner, OrderType::WalkingUpright));
    let mut finished = movement_elem(owner, OrderType::WalkingUpright);
    finished.state = SequenceState::Terminated;
    finished.push_order(Order::test_new(OrderType::WalkingUpright, 0.0, 0.0));
    sequence.append_element(finished);
    let sequence_id = engine.orders.sequence_manager.launch_sequence(sequence);
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), sequence_id, 0);

    engine.orders.sequence_manager.make_fast(owner);

    let follower = engine
        .orders
        .sequence_manager
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
    for element in s.elements.iter().take(2) {
        let SequenceElementData::Movement { action, .. } = element.data else {
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
    let (mut engine, assets, owner) = live_sequence_fixture();
    let mut sequence = Sequence::new();
    for _ in 0..3 {
        sequence.append_element(SequenceElement::new(1, Command::Generic, Some(owner)));
    }
    let sequence_id = engine.orders.sequence_manager.launch_sequence(sequence);
    engine
        .orders
        .sequence_manager
        .get_element_mut(sequence_id, 0)
        .unwrap()
        .legacy_v48 = Some(loaded_v48_state(Some(SequenceElementRef::new(
        sequence_id,
        2,
    ))));
    EngineInner::with_condolation_callback(
        move |engine, card| {
            if card.elem_idx == 0 {
                assert_eq!(
                    engine
                        .orders
                        .sequence_manager
                        .get_element(sequence_id, 2)
                        .unwrap()
                        .state,
                    SequenceState::Todo
                );
            }
        },
        || {
            engine.element_interrupted(
                &test_context(),
                &assets,
                &mut Vec::new(),
                sequence_id,
                0,
                CascadeFlags::FOLLOWING,
            )
        },
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 1)
            .unwrap()
            .state,
        SequenceState::Todo
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 2)
            .unwrap()
            .state,
        SequenceState::Interrupted
    );
}

#[test]
fn loaded_movement_interruption_reaches_cross_sequence_linked_seek() {
    let (mut engine, assets, owner) = live_sequence_fixture();
    let linked_id = engine
        .orders
        .sequence_manager
        .launch_element(movement_elem(owner, OrderType::WalkingUpright));
    let mut movement = movement_elem(owner, OrderType::WalkingUpright);
    movement.command = Command::MoveWaiting;
    let movement_id = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .get_element_mut(movement_id, 0)
        .unwrap()
        .legacy_v48 = Some(LegacyV48SequenceElementState {
        linked_seek: Some(Some(SequenceElementRef::new(linked_id, 0))),
        ..loaded_v48_state(None)
    });
    let cards = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = cards.clone();
    EngineInner::with_condolation_callback(
        move |engine, card| {
            observed.borrow_mut().push(card.seq_id);
            if card.seq_id == linked_id {
                let parent = engine
                    .orders
                    .sequence_manager
                    .get_element(movement_id, 0)
                    .unwrap();
                assert_eq!(parent.command, Command::Move);
                assert_eq!(
                    parent.state,
                    SequenceState::Todo,
                    "the linked callback completes before its parent changes state"
                );
            }
        },
        || {
            engine.element_interrupted(
                &test_context(),
                &assets,
                &mut Vec::new(),
                movement_id,
                0,
                CascadeFlags::NEXT_LEVEL,
            )
        },
    );
    assert_eq!(*cards.borrow(), [linked_id, movement_id]);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(linked_id, 0)
            .unwrap()
            .state,
        SequenceState::Interrupted
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(movement_id, 0)
            .unwrap()
            .state,
        SequenceState::Interrupted
    );
}

#[test]
fn loaded_nonadjacent_next_controls_stop_recursion() {
    let (mut engine, assets, owner) = live_sequence_fixture();
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
    let sequence_id = engine.orders.sequence_manager.launch_sequence(sequence);
    engine
        .orders
        .sequence_manager
        .get_element_mut(sequence_id, 0)
        .unwrap()
        .legacy_v48 = Some(loaded_v48_state(Some(SequenceElementRef::new(
        sequence_id,
        2,
    ))));
    engine.stop_owner_current_from_root(
        &test_context(),
        &assets,
        &mut Vec::new(),
        owner,
        Some((sequence_id, 0)),
        SequencePriority::Normal,
        &|_, element| element.priority,
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 1)
            .unwrap()
            .state,
        SequenceState::Todo
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 2)
            .unwrap()
            .state,
        SequenceState::Interrupted
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .unwrap()
            .legacy_v48
            .as_ref()
            .unwrap()
            .next,
        None
    );
}

#[test]
fn stop_movement_rewrites_order_and_shortens_only_element_destination() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let mut seq = Sequence::new();
    let mut elem = SequenceElement::new_movement(
        1,
        Command::Move,
        Some(fixture_owner_0),
        OrderType::WalkingUpright,
    );
    elem.push_order(Order::test_new(OrderType::WalkingUpright, 100.0, 0.0));
    elem.push_order(Order::test_new(OrderType::WalkingUpright, 200.0, 0.0));
    seq.append_element(elem);
    let seq_id = engine.orders.sequence_manager.launch_sequence(seq);

    // Advance to InProgress so stop_movement_for_owner applies.
    let _ = engine.orders.sequence_manager.hourglass();
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), seq_id, 0);

    engine.orders.pending_path_requests =
        crate::engine::PendingPathRequestQueue::restore_v48_waiting(vec![
            crate::engine::PendingPathRequest::test_request(fixture_owner_0, seq_id, 0),
        ]);
    let changed = engine.stop_movement_for_owner(
        &sim,
        &assets,
        &mut Vec::new(),
        fixture_owner_0,
        crate::coordinates::MapPoint { x: 0.0, y: 0.0 },
        SequencePriority::NonInterruptable,
        &|_, _| SequencePriority::Normal,
    );
    assert!(changed);
    let s = engine.orders.sequence_manager.get_sequence(seq_id).unwrap();
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
    assert_eq!(engine.orders.pending_path_requests.v48_waiting().len(), 1);

    // The generic half of actor stopping runs after movement stopping. The original game
    // leaves this rewritten movement InProgress so the transition can
    // play; only movement actions without a rewrite arm were interrupted
    // above.
    engine.stop_owner(
        &sim,
        &assets,
        &mut Vec::new(),
        fixture_owner_0,
        SequencePriority::NonInterruptable,
        &|_, _| SequencePriority::Normal,
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );
}

#[test]
fn stop_movement_rewrite_does_not_cancel_path_for_move_waiting() {
    // Path cancellation only fires when the element is pushed to
    // `Interrupted` (default switch branch).  A successful rewrite
    // keeps the element in INPROGRESS and the path request stays
    // alive.
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let mut seq = Sequence::new();
    let mut elem = SequenceElement::new_movement(
        1,
        Command::MoveWaiting,
        Some(fixture_owner_0),
        OrderType::WalkingUpright,
    );
    elem.push_order(Order::test_new(OrderType::WalkingUpright, 100.0, 0.0));
    seq.append_element(elem);
    let seq_id = engine.orders.sequence_manager.launch_sequence(seq);
    let _ = engine.orders.sequence_manager.hourglass();
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), seq_id, 0);

    engine.orders.pending_path_requests =
        crate::engine::PendingPathRequestQueue::restore_v48_waiting(vec![
            crate::engine::PendingPathRequest::test_request(fixture_owner_0, seq_id, 0),
        ]);
    engine.stop_movement_for_owner(
        &sim,
        &assets,
        &mut Vec::new(),
        fixture_owner_0,
        crate::coordinates::MapPoint::default(),
        SequencePriority::NonInterruptable,
        &|_, _| SequencePriority::Normal,
    );
    assert_eq!(engine.orders.pending_path_requests.v48_waiting().len(), 1);
    let s = engine.orders.sequence_manager.get_sequence(seq_id).unwrap();
    assert_eq!(s.elements[0].command, Command::MoveWaiting);
}

#[test]
fn stop_movement_cancels_path_on_interrupt() {
    // With an action that has no waiting-transition variant, the
    // element falls into the default branch and gets interrupted;
    // a MoveWaiting command goes through path cancellation.
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let mut seq = Sequence::new();
    let mut elem = SequenceElement::new_movement(
        1,
        Command::MoveWaiting,
        Some(fixture_owner_0),
        OrderType::Turning,
    );
    elem.push_order(Order::test_new(OrderType::Turning, 100.0, 0.0));
    seq.append_element(elem);
    let seq_id = engine.orders.sequence_manager.launch_sequence(seq);
    let _ = engine.orders.sequence_manager.hourglass();
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), seq_id, 0);

    engine.orders.pending_path_requests =
        crate::engine::PendingPathRequestQueue::restore_v48_waiting(vec![
            crate::engine::PendingPathRequest::test_request(fixture_owner_0, seq_id, 0),
        ]);
    engine.stop_movement_for_owner(
        &sim,
        &assets,
        &mut Vec::new(),
        fixture_owner_0,
        crate::coordinates::MapPoint::default(),
        SequencePriority::NonInterruptable,
        &|_, _| SequencePriority::Normal,
    );
    assert_eq!(engine.orders.pending_path_requests.v48_waiting().len(), 1);
    assert_eq!(
        serde_json::to_value(&engine.orders.pending_path_requests).unwrap()["ignore_next_path"],
        true
    );
    let s = engine.orders.sequence_manager.get_sequence(seq_id).unwrap();
    assert_eq!(s.elements[0].command, Command::Move);
    assert_eq!(s.elements[0].state, SequenceState::Interrupted);
}

#[test]
fn stop_movement_interrupts_element_with_unknown_action() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let mut seq = Sequence::new();
    let mut elem =
        SequenceElement::new_movement(1, Command::Move, Some(fixture_owner_0), OrderType::Turning);
    elem.push_order(Order::test_new(OrderType::Turning, 100.0, 0.0));
    seq.append_element(elem);
    let seq_id = engine.orders.sequence_manager.launch_sequence(seq);
    let _ = engine.orders.sequence_manager.hourglass();
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), seq_id, 0);

    engine.orders.pending_path_requests =
        crate::engine::PendingPathRequestQueue::restore_v48_waiting(vec![
            crate::engine::PendingPathRequest::test_request(fixture_owner_0, seq_id, 0),
        ]);
    engine.stop_movement_for_owner(
        &sim,
        &assets,
        &mut Vec::new(),
        fixture_owner_0,
        crate::coordinates::MapPoint::default(),
        SequencePriority::NonInterruptable,
        &|_, _| SequencePriority::Normal,
    );
    let s = engine.orders.sequence_manager.get_sequence(seq_id).unwrap();
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
fn following_queries_preserve_cross_sequence_and_owner_policies() {
    let owner = EntityId::Pc(crate::entity_id::PcId(1));
    let other = EntityId::Pc(crate::entity_id::PcId(2));
    let mut mgr = SequenceManager::new();
    let root = mgr.launch_element(movement_elem(owner, OrderType::RunningUpright));
    let next = mgr.launch_element(movement_elem(owner, OrderType::RunningUpright));
    mgr.get_element_mut(root, 0).unwrap().legacy_v48 =
        Some(loaded_v48_state(Some(SequenceElementRef::new(next, 0))));

    assert_eq!(mgr.next_element_in_chain(root, 0), Some((next, 0)));
    assert!(!mgr.is_last_real_action(root, 0));
    mgr.set_action_recursive(root, 0, OrderType::WalkingCrouched);
    assert_eq!(
        movement_action(mgr.get_element(next, 0).unwrap()),
        OrderType::WalkingCrouched
    );

    mgr.get_element_mut(next, 0).unwrap().owner = Some(other);
    assert_eq!(mgr.next_element_in_chain(root, 0), None);
    assert!(!mgr.is_last_real_action(root, 0));
    mgr.set_action_recursive(root, 0, OrderType::RunningUpright);
    assert_eq!(
        movement_action(mgr.get_element(next, 0).unwrap()),
        OrderType::WalkingCrouched
    );
}

#[test]
fn graph_rewrites_preserve_stored_edges_hidden_from_live_queries() {
    let owner = EntityId::Pc(crate::entity_id::PcId(1));
    let mut sequence = Sequence::new();
    sequence.append_element(movement_elem(owner, OrderType::RunningUpright));
    sequence.append_element(movement_elem(owner, OrderType::RunningUpright));
    let mut mgr = SequenceManager::new();
    let id = mgr.launch_sequence(sequence);
    mgr.get_element_mut(id, 0).unwrap().next_link_severed = true;

    assert_eq!(mgr.following_element_ref(id, 0), None);
    assert_eq!(mgr.next_element_in_chain(id, 0), None);
    assert!(mgr.is_last_real_action(id, 0));
    assert_eq!(mgr.rewrite_following_ref(id, 0), Some((id, 1)));
    mgr.set_action_recursive(id, 0, OrderType::WalkingCrouched);
    assert_eq!(
        movement_action(mgr.get_element(id, 1).unwrap()),
        OrderType::WalkingCrouched
    );
}

#[test]
#[should_panic(expected = "following pointer crosses sequences")]
fn local_following_query_rejects_cross_sequence_link() {
    let mut mgr = SequenceManager::new();
    let root = mgr.launch_element(SequenceElement::new(1, Command::Wait, None));
    let next = mgr.launch_element(SequenceElement::new(1, Command::Wait, None));
    mgr.get_element_mut(root, 0).unwrap().legacy_v48 =
        Some(loaded_v48_state(Some(SequenceElementRef::new(next, 0))));
    mgr.following_element_ref(root, 0);
}

#[test]
#[should_panic(expected = "following pointer targets missing element")]
fn local_following_query_rejects_dangling_link() {
    let mut sequence = Sequence::new();
    sequence.append_element(SequenceElement::new(1, Command::Wait, None));
    sequence.elements[0].legacy_v48 = Some(loaded_v48_state(Some(SequenceElementRef::new(
        sequence.id,
        7,
    ))));
    sequence.following_element_index(0);
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
        "the original game follows the next-element link without an owner-identity gate"
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
        "deferred facing must not restore a movement goal cleared by the outgoing notification"
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
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;

    let mut todo = SequenceElement::new(1, Command::LeaveListen, Some(owner));
    todo.priority = SequencePriority::NonInterruptable;
    let todo_seq = engine.orders.sequence_manager.launch_element(todo);
    engine.element_impossible(&sim, &assets, &mut Vec::new(), todo_seq, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(todo_seq, 0)
            .unwrap()
            .state,
        SequenceState::Impossible,
        "preflight failure must reject a Todo non-interruptable element"
    );

    let mut active = SequenceElement::new(1, Command::EnterListen, Some(owner));
    active.priority = SequencePriority::NonInterruptable;
    let active_seq = engine.orders.sequence_manager.launch_element(active);
    engine.element_in_progress(&sim, &assets, &mut Vec::new(), active_seq, 0);
    engine.element_impossible(&sim, &assets, &mut Vec::new(), active_seq, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(active_seq, 0)
            .unwrap()
            .state,
        SequenceState::InProgress,
        "an executing non-interruptable owner remains protected"
    );

    engine.element_impossible_from_execute(&sim, &assets, &mut Vec::new(), active_seq, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(active_seq, 0)
            .unwrap()
            .state,
        SequenceState::Impossible,
        "the owner's intrinsic execution abort follows the game's behavior"
    );
}

#[test]
fn death_cleanup_preserves_exact_dead_human_todo_whitelist() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;

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
        let sequence = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, command, Some(owner)));
        (command, sequence)
    });
    let rejected = [Command::ReceiveStoneDamage, Command::WaitTimer].map(|command| {
        let sequence = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, command, Some(owner)));
        (command, sequence)
    });

    engine.kill_owner_sequences(&sim, &assets, &mut Vec::new(), owner, SequenceId(u32::MAX));

    for (command, sequence) in admitted {
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .state,
            SequenceState::Todo,
            "dead-human instruction handling admits queued {command:?}"
        );
    }
    for (command, sequence) in rejected {
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .state,
            SequenceState::Interrupted,
            "dead-human instruction handling rejects queued {command:?}"
        );
    }
}

#[test]
fn death_cleanup_preserves_postponed_wait_transferred_to_damage_replacement() {
    let (mut engine, mut assets, fixture_owner_0) = live_sequence_fixture();
    let sim = test_context();

    let owner = fixture_owner_0;

    let damage = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::ReceiveDamage, Some(owner)));
    let wait = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::Wait, Some(owner)));
    engine.postpone_element(&sim, &assets, &mut Vec::new(), wait, 0);
    engine
        .orders
        .sequence_manager
        .get_element_mut(damage, 0)
        .unwrap()
        .cross_postponed = Some((wait, 0));

    let rejected = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::WaitTimer, Some(owner)));
    engine.postpone_element(&sim, &assets, &mut Vec::new(), rejected, 0);

    engine.kill_owner_sequences(&sim, &assets, &mut Vec::new(), owner, damage);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(wait, 0)
            .unwrap()
            .state,
        SequenceState::Postponed,
        "human death leaves the dead-admissible Wait queued behind lethal damage"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(damage, 0)
            .unwrap()
            .cross_postponed,
        Some((wait, 0)),
        "death cleanup must preserve the replacement's transferred postponed chain"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(rejected, 0)
            .unwrap()
            .state,
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
    seek.point_seek_route_provenance = PointSeekRouteProvenance::OriginalReplay;
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
            door_index: crate::gate::DoorIndex::new(7).expect("valid door index"),
            direct: false,
        }]),
    };
    let mut manager = SequenceManager::new();

    assert!(
        manager
            .inject_recorded_drop_ale_route(
                owner,
                destination,
                authoritative_sector,
                0,
                route.clone(),
            )
            .is_err(),
        "an event arriving before its command must not mutate unrelated state"
    );
    let sequence =
        manager.launch_element(pending_drop_ale_seek(owner, destination, fallback_sector));
    let unrelated_destination = crate::coordinates::MapPoint::new(779.0, 1714.0);
    assert!(!manager.has_pending_drop_ale_route_candidate(owner, unrelated_destination));
    assert!(
        manager
            .inject_recorded_drop_ale_route(
                owner,
                unrelated_destination,
                authoritative_sector,
                0,
                route.clone(),
            )
            .is_err(),
        "a same-actor route with different goal bits must remain unrelated"
    );
    assert!(
        manager
            .inject_recorded_drop_ale_route(
                owner,
                destination,
                authoritative_sector,
                0,
                route.clone(),
            )
            .is_ok()
    );
    let element = manager.get_element(sequence, 0).unwrap();
    let SequenceElementData::Movement { sector, layer, .. } = &element.data else {
        unreachable!()
    };
    assert_eq!(*sector, Some(authoritative_sector));
    assert_eq!(*layer, 0);
    assert_eq!(element.recorded_gate_path, Some(route));

    manager.get_element_mut(sequence, 0).unwrap().state = SequenceState::Terminated;
    assert!(
        manager
            .inject_recorded_drop_ale_route(
                owner,
                destination,
                fallback_sector,
                2,
                recorded_drop_ale_failure(),
            )
            .is_err(),
        "terminal DropAle elements must not receive later route events"
    );
}

#[test]
fn delayed_drop_ale_route_rejects_multiple_pending_matches() {
    let owner = EntityId::Pc(crate::entity_id::PcId(36));
    let destination = crate::coordinates::MapPoint::new(778.0, 1714.0);
    let fallback_sector = crate::position_interface::SectorHandle::new(25).unwrap();
    let mut manager = SequenceManager::new();
    manager.launch_element(pending_drop_ale_seek(owner, destination, fallback_sector));
    manager.launch_element(pending_drop_ale_seek(owner, destination, fallback_sector));

    let error = manager
        .inject_recorded_drop_ale_route(
            owner,
            destination,
            crate::position_interface::SectorHandle::new(0).unwrap(),
            0,
            recorded_drop_ale_failure(),
        )
        .expect_err("ambiguous delayed route must fail");
    assert!(error.contains("matched 2 pending point Seeks"));
}

#[test]
#[should_panic(expected = "already has a recorded gate route")]
fn delayed_drop_ale_route_rejects_a_second_exact_route_event() {
    let owner = EntityId::Pc(crate::entity_id::PcId(36));
    let destination = crate::coordinates::MapPoint::new(778.0, 1714.0);
    let fallback_sector = crate::position_interface::SectorHandle::new(25).unwrap();
    let mut manager = SequenceManager::new();
    manager.launch_element(pending_drop_ale_seek(owner, destination, fallback_sector));
    assert!(
        manager
            .inject_recorded_drop_ale_route(
                owner,
                destination,
                crate::position_interface::SectorHandle::new(0).unwrap(),
                0,
                recorded_drop_ale_failure(),
            )
            .is_ok()
    );

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
    assert!(
        manager
            .inject_recorded_drop_ale_route(
                owner,
                destination,
                crate::position_interface::SectorHandle::new(0).unwrap(),
                0,
                route.clone(),
            )
            .is_ok()
    );
    let encoded = bitcode::encode(&route);
    let decoded_route: crate::gate::RecordedGatePath =
        bitcode::decode(&encoded).expect("decode recorded route metadata");
    assert_eq!(decoded_route, route);

    let element_json = serde_json::to_string(manager.get_element(sequence, 0).unwrap())
        .expect("encode complete pending DropAle element");
    let decoded_element: SequenceElement =
        serde_json::from_str(&element_json).expect("decode complete pending DropAle element");
    assert_eq!(decoded_element.recorded_gate_path, Some(route));
    assert_eq!(
        decoded_element.point_seek_route_provenance,
        PointSeekRouteProvenance::OriginalReplay,
    );
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

    let mut missing_provenance = serde_json::to_value(
        manager
            .get_element(sequence, 0)
            .expect("pending DropAle element remains present"),
    )
    .expect("encode pending DropAle element as JSON value");
    assert!(
        missing_provenance
            .as_object_mut()
            .expect("SequenceElement JSON is an object")
            .remove("point_seek_route_provenance")
            .is_some(),
        "current SequenceElement serialization must include route provenance"
    );
    let error = serde_json::from_value::<SequenceElement>(missing_provenance)
        .expect_err("current Rust SequenceElement JSON must require route provenance");
    assert!(
        error.to_string().contains("point_seek_route_provenance"),
        "missing-provenance error named the wrong field: {error}"
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
