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
