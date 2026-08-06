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
