use super::*;
use crate::element::{ActionState, Posture};
use crate::order::OrderType;
use crate::sequence::{SequenceElement, SequencePriority, SequenceState};
use crate::sprite::MotionState;

#[test]
fn retained_shot_refreshes_transition_state_when_aiming_resumes() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let sim = crate::sim_rng::test_context();
    let owner = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
        Posture::Upright,
    ));
    let entity = engine.get_entity_mut(owner).unwrap();
    entity.actor_data_mut().unwrap().action_state = ActionState::AimingWithBow;
    entity.element_data_mut().sprite.last_action = OrderType::TransitionLoadingBow;
    let mut shot = SequenceElement::new_interaction(1, Command::ShootBow, Some(owner), None);
    shot.priority = SequencePriority::Wait;
    // Retained work may still carry transition operands from an earlier admission.
    shot.posture_after_transition = Posture::Sitting;
    shot.action_state_after_transition = ActionState::Waiting;
    let sequence = engine.launch_element(&sim, &assets, shot);

    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .sprite
        .last_action = OrderType::AimingWithBow;
    engine.process_shoot_list_for(&sim, &assets, owner);

    let shot = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap();
    assert_eq!(shot.posture_after_transition, Posture::Upright);
    assert_eq!(
        shot.action_state_after_transition,
        ActionState::AimingWithBow
    );
    assert_eq!(shot.state, SequenceState::Impossible);
    assert!(
        engine
            .get_entity(owner)
            .unwrap()
            .human_data()
            .unwrap()
            .pending_shoots
            .is_empty()
    );
}

#[test]
fn held_bow_instruction_unfreezes_before_retaining_the_shot() {
    for priority in [SequencePriority::Wait, SequencePriority::Normal] {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let sim = crate::sim_rng::test_context();
        let owner = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
            Posture::Upright,
        ));
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.actor_data_mut().unwrap().execution_frozen = true;
        entity.element_data_mut().sprite.last_action = OrderType::TransitionLoadingBow;
        let mut shot = SequenceElement::new_interaction(1, Command::ShootBow, Some(owner), None);
        shot.priority = priority;

        let sequence = engine.launch_element(&sim, &assets, shot);
        if priority == SequencePriority::Normal {
            engine.hourglass_phase_sequences(&sim, &mut HostDisplayState::default(), &assets);
        }

        let entity = engine.get_entity(owner).unwrap();
        assert!(!entity.actor_data().unwrap().execution_frozen);
        assert_eq!(
            entity.human_data().unwrap().pending_shoots,
            [crate::sequence::SequenceElementRef::new(sequence, 0)]
        );
        let held = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap();
        assert_eq!(held.state, SequenceState::Todo);
        assert!(held.orders.is_empty());
        assert_eq!(held.posture_after_transition, Posture::Undefined);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_element_for_actor(owner),
            None,
            "loading retains the shot without translating or selecting it"
        );
    }
}

#[test]
fn whistle_translation_is_identical_for_immediate_and_registered_instructions() {
    for priority in [SequencePriority::Wait, SequencePriority::Normal] {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let sim = crate::sim_rng::test_context();
        let owner = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
            Posture::Upright,
        ));
        let actor = engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.action_state = ActionState::Waiting;
        actor.continuation.motion_state = MotionState::Terminated;
        let mut whistle = SequenceElement::new(1, Command::WhistleCmd, Some(owner));
        whistle.priority = priority;

        let sequence = engine.launch_element(&sim, &assets, whistle);
        if priority == SequencePriority::Normal {
            assert_eq!(
                engine
                    .orders
                    .sequence_manager
                    .current_element_for_actor(owner),
                None,
                "registered instructions must wait for the sequence phase"
            );
            engine.hourglass_phase_sequences(&sim, &mut HostDisplayState::default(), &assets);
        }

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_element_for_actor(owner),
            Some((sequence, 0))
        );
        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::InProgress);
        assert_eq!(
            element
                .orders
                .iter()
                .map(|order| order.order_type)
                .collect::<Vec<_>>(),
            vec![OrderType::Whistling]
        );
        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        assert_eq!(actor.whistle_wait_time, 25);
        assert_eq!(actor.continuation.motion_state, MotionState::InProgress);
    }
}

#[test]
fn completion_during_translation_does_not_latch_instruction_motion() {
    for priority in [SequencePriority::Wait, SequencePriority::Normal] {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let sim = crate::sim_rng::test_context();
        let owner = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
            Posture::Upright,
        ));
        let actor = engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.action_state = ActionState::Waiting;
        actor.continuation.motion_state = MotionState::Terminated;
        let mut assertion = SequenceElement::new_movement(
            1,
            Command::AssertPosition,
            Some(owner),
            OrderType::WalkingUpright,
        );
        assertion.priority = priority;

        let sequence = engine.launch_element(&sim, &assets, assertion);
        if priority == SequencePriority::Normal {
            engine.hourglass_phase_sequences(&sim, &mut HostDisplayState::default(), &assets);
        }

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .state,
            SequenceState::Terminated
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_element_for_actor(owner),
            None
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .continuation
                .motion_state,
            MotionState::Terminated,
            "a translation that clears selection must skip the ordinary instruction epilogue"
        );
    }
}
