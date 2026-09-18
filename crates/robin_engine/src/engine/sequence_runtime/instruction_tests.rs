use super::*;
use crate::element::{ActionState, Posture};
use crate::order::OrderType;
use crate::sequence::{SequenceElement, SequencePriority, SequenceState};
use crate::sprite::MotionState;

#[test]
fn same_building_seek_keeps_synchronously_launched_post_seek_selection() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let sim = crate::sim_rng::test_context();
    let owner = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
        Posture::Upright,
    ));
    let target = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
        Posture::Upright,
    ));
    for actor in [owner, target] {
        engine
            .elem_mut(actor)
            .set_sector(crate::position_interface::SectorHandle::new(1));
    }
    let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
    level.map_bbox = crate::coordinates::MapBBox::from_coords(-100.0, -100.0, 100.0, 100.0);
    level
        .sector_number_map
        .insert(crate::sector::SectorNumber::new(1), 0);
    level.sectors.push(crate::fast_find_grid::GridSector {
        bounding_box: level.map_bbox,
        sector_type: crate::sector::SectorType::BUILDING,
        sector_number: crate::sector::SectorNumber::new(1),
        ..Default::default()
    });
    let replacement_order = engine.orders.allocate_order_id();
    let mut replacement = SequenceElement::new(1, Command::Generic, Some(owner));
    replacement.priority = SequencePriority::Wait;
    replacement.orders.push_back(crate::order::Order::new(
        OrderType::WaitingUpright,
        0.0,
        0.0,
        replacement_order,
    ));
    let mut post_seek = crate::sequence::Sequence::new();
    post_seek.append_element(replacement);
    let mut seek =
        SequenceElement::new_movement(1, Command::Seek, Some(owner), OrderType::WalkingUpright);
    if let crate::sequence::SequenceElementData::Movement {
        element,
        flags,
        post_seek_sequence,
        ..
    } = &mut seek.data
    {
        *element = Some(target);
        *flags = crate::sequence::MoveFlags::SEEK | crate::sequence::MoveFlags::SEEK_IN_BUILDINGS;
        *post_seek_sequence = Some(post_seek.into_post_seek());
    }
    let sequence = engine.orders.sequence_manager.insert_element(seek);
    engine
        .orders
        .sequence_manager
        .start_sequence_level(sequence);
    assert!(engine.instruct_owner(&sim, &assets, &mut Vec::new(), owner, sequence, 0));
    let selected = engine
        .world
        .entities
        .current_element_for_actor(owner)
        .expect("post-seek carrier remains selected");
    assert_ne!(selected, (sequence, 0));
    assert_eq!(
        engine
            .actor(owner)
            .installed_order
            .unwrap()
            .resolve(&engine.orders.sequence_manager)
            .order_id,
        replacement_order
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
}

#[test]
fn halt_keeps_selection_order_and_goal_installed_by_termination_callback() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let sim = crate::sim_rng::test_context();
    let owner = engine.add_test_entity(
        crate::engine::test_support::actors::TestActor::soldier(Posture::Upright).build(),
    );
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    let mut make_carrier = || {
        let order_id = engine.orders.allocate_order_id();
        let mut element = SequenceElement::new(1, Command::Generic, Some(owner));
        element.priority = SequencePriority::Normal;
        element.orders.push_back(crate::order::Order::new(
            OrderType::WaitingUpright,
            0.0,
            0.0,
            order_id,
        ));
        let sequence = engine.orders.sequence_manager.insert_element(element);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence);
        (sequence, order_id)
    };
    let (outgoing, _) = make_carrier();
    let (nested, nested_order) = make_carrier();
    assert!(engine.instruct_owner(&sim, &assets, &mut Vec::new(), owner, outgoing, 0));
    let mut pending = SequenceElement::new(1, Command::Generic, Some(owner));
    pending.priority = SequencePriority::Normal;
    let pending = engine.t_launch_element(&assets, pending);
    let goal = crate::coordinates::MapPoint::new(23.0, 41.0);
    let callback_assets = assets.clone();
    EngineInner::with_condolation_callback(
        move |engine, card| {
            if card.seq_id == pending {
                assert_eq!(engine.world.entities.current_element_for_actor(owner), None);
                assert!(engine.instruct_owner(
                    &crate::sim_rng::test_context(),
                    &callback_assets,
                    &mut Vec::new(),
                    owner,
                    nested,
                    0
                ));
                engine
                    .ent_mut(owner)
                    .position_iface_mut()
                    .set_map_goal(goal);
            }
        },
        || engine.halt_actor(&sim, &assets, owner),
    );
    assert_eq!(
        engine.world.entities.current_element_for_actor(owner),
        Some((nested, 0))
    );
    let entity = engine.ent(owner);
    assert_eq!(
        entity
            .actor_data()
            .unwrap()
            .installed_order
            .unwrap()
            .resolve(&engine.orders.sequence_manager)
            .order_id,
        nested_order
    );
    assert_eq!(entity.position_iface().map_goal(), goal);
}

#[test]
fn nested_instruction_selection_survives_outer_callback_return() {
    for nested_has_order in [false, true] {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let sim = crate::sim_rng::test_context();
        let owner = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
            Posture::Upright,
        ));
        let mut insert = |has_order| {
            let mut element = SequenceElement::new(1, Command::Generic, Some(owner));
            element.priority = SequencePriority::Normal;
            if has_order {
                element.orders.push_back(crate::order::Order::new(
                    OrderType::WaitingUpright,
                    0.0,
                    0.0,
                    engine.orders.allocate_order_id(),
                ));
            }
            let sequence = engine.orders.sequence_manager.insert_element(element);
            engine
                .orders
                .sequence_manager
                .start_sequence_level(sequence);
            sequence
        };
        let outgoing = insert(true);
        let incoming = insert(true);
        let nested = insert(nested_has_order);
        assert!(engine.instruct_owner(&sim, &assets, &mut Vec::new(), owner, outgoing, 0));
        EngineInner::with_condolation_callback(
            move |engine, card| {
                if card.seq_id == outgoing {
                    assert_eq!(
                        engine.world.entities.current_element_for_actor(owner),
                        Some((incoming, 0))
                    );
                    assert!(engine.instruct_owner(
                        &crate::sim_rng::test_context(),
                        &LevelAssets::new(),
                        &mut Vec::new(),
                        owner,
                        nested,
                        0,
                    ));
                }
            },
            || {
                assert!(!engine.instruct_owner(&sim, &assets, &mut Vec::new(), owner, incoming, 0));
            },
        );
        assert_eq!(
            engine.world.entities.current_element_for_actor(owner),
            nested_has_order.then_some((nested, 0)),
        );
        let actor = engine.actor(owner);
        assert_eq!(actor.installed_order.is_some(), nested_has_order);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(incoming, 0)
                .unwrap()
                .state,
            SequenceState::Interrupted,
        );
    }
}

#[test]
fn retained_shot_refreshes_transition_state_when_aiming_resumes() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let sim = crate::sim_rng::test_context();
    let owner = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
        Posture::Upright,
    ));
    let entity = engine.ent_mut(owner);
    entity.actor_data_mut().unwrap().action_state = ActionState::AimingWithBow;
    entity.element_data_mut().sprite.last_action = OrderType::TransitionLoadingBow;
    let mut shot = SequenceElement::new_interaction(1, Command::ShootBow, Some(owner), None);
    shot.priority = SequencePriority::Wait;
    // Retained work may still carry transition operands from an earlier admission.
    shot.posture_after_transition = Posture::Sitting;
    shot.action_state_after_transition = ActionState::Waiting;
    let sequence = engine.t_launch_element(&assets, shot);

    engine.elem_mut(owner).sprite.last_action = OrderType::AimingWithBow;
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
    assert!(engine.human(owner).pending_shoots.is_empty());
}

#[test]
fn held_bow_instruction_unfreezes_before_retaining_the_shot() {
    for priority in [SequencePriority::Wait, SequencePriority::Normal] {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
            Posture::Upright,
        ));
        let entity = engine.ent_mut(owner);
        entity.actor_data_mut().unwrap().execution_frozen = true;
        entity.element_data_mut().sprite.last_action = OrderType::TransitionLoadingBow;
        let mut shot = SequenceElement::new_interaction(1, Command::ShootBow, Some(owner), None);
        shot.priority = priority;

        let sequence = engine.t_launch_element(&assets, shot);
        if priority == SequencePriority::Normal {
            engine.t_hourglass_phase_sequences(&assets);
        }

        let entity = engine.ent(owner);
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
            engine.world.entities.current_element_for_actor(owner),
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
        let owner = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
            Posture::Upright,
        ));
        let actor = engine.actor_mut(owner);
        actor.action_state = ActionState::Waiting;
        actor.continuation.motion_state = MotionState::Terminated;
        let mut whistle = SequenceElement::new(1, Command::WhistleCmd, Some(owner));
        whistle.priority = priority;

        let sequence = engine.t_launch_element(&assets, whistle);
        if priority == SequencePriority::Normal {
            assert_eq!(
                engine.world.entities.current_element_for_actor(owner),
                None,
                "registered instructions must wait for the sequence phase"
            );
            engine.t_hourglass_phase_sequences(&assets);
        }

        assert_eq!(
            engine.world.entities.current_element_for_actor(owner),
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
        let actor = engine.actor(owner);
        assert_eq!(actor.wait_time, 25);
        assert_eq!(actor.continuation.motion_state, MotionState::InProgress);
    }
}

#[test]
fn completion_during_translation_does_not_latch_instruction_motion() {
    for priority in [SequencePriority::Wait, SequencePriority::Normal] {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
            Posture::Upright,
        ));
        let actor = engine.actor_mut(owner);
        actor.action_state = ActionState::Waiting;
        actor.continuation.motion_state = MotionState::Terminated;
        let mut assertion = SequenceElement::new_movement(
            1,
            Command::AssertPosition,
            Some(owner),
            OrderType::WalkingUpright,
        );
        assertion.priority = priority;

        let sequence = engine.t_launch_element(&assets, assertion);
        if priority == SequencePriority::Normal {
            engine.t_hourglass_phase_sequences(&assets);
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
        assert_eq!(engine.world.entities.current_element_for_actor(owner), None);
        assert_eq!(
            engine.motion_state_of(owner),
            MotionState::Terminated,
            "a translation that clears selection must skip the ordinary instruction epilogue"
        );
    }
}
