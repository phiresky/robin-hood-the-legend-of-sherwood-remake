use super::*;

pub(super) fn bind_test_action_point(
    engine: &mut EngineInner,
    id: EntityId,
    action: crate::order::OrderType,
    hotspot: crate::coordinates::SpriteLocalPoint,
    center: crate::coordinates::SpriteAnchor,
) {
    let script = crate::sprite_script::SpriteScript {
        action_id: action as u16,
        action_done: 0,
        average_speed: 0.0,
        hotspot,
        sum_distance: 0,
        frame_ids: vec![1],
        delays: vec![1],
        distances: vec![0],
        offsets: vec![SpriteFrameOffset::ZERO],
        sound_ids: vec![0],
    };
    let mut conversion =
        vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
    conversion[action as usize] = 0;
    let mut sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(conversion),
    );
    sprite.center = center;
    let element = engine.get_entity_mut(id).unwrap().element_data_mut();
    let position = element.position_map();
    let direction = element.direction();
    element.sprite = sprite;
    element.set_position_map(position);
    element.set_direction_instantly(direction);
}

pub(super) fn bind_test_bow_release_action(engine: &mut EngineInner, id: EntityId) {
    let action = crate::order::OrderType::ShootingWithBow;
    let script = crate::sprite_script::SpriteScript {
        action_id: action as u16,
        action_done: 0,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::new(2.0, 3.0),
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0, 0, 0],
    };
    let mut conversion =
        vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
    conversion[action as usize] = 0;
    let mut sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    let element = engine.get_entity_mut(id).unwrap().element_data_mut();
    let position = element.position_map();
    let direction = element.direction();
    sprite.center = crate::coordinates::SpriteAnchor::ZERO;
    element.sprite = sprite;
    element.set_position_map(position);
    element.set_direction_instantly(direction);
}

#[test]
fn postponed_generic_order_carrier_resumes_in_progress() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    let mut element = SequenceElement::new_generic(1, Command::Generic, Some(soldier));
    element.posture_after_transition = Posture::Upright;
    element.orders.push_back(Order::new(
        OrderType::WaitingUpright,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    let sequence = engine.orders.sequence_manager.launch_element(element);

    let mut display = HostDisplayState::default();
    let assets = LevelAssets::default();
    engine.hourglass_phase_sequences(&crate::sim_rng::test_context(), &mut display, &assets);

    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("generic carrier remains live while its order plays");
    assert_eq!(element.state, SequenceState::InProgress);
    assert_eq!(element.orders.len(), 1);
}

#[test]
fn retained_waiting_sword_handoff_preserves_running_sprite_identity() {
    use crate::element::{Command, InstalledActorOrder, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    let old_order_id = engine.orders.allocate_order_id();
    let new_order_id = engine.orders.allocate_order_id();
    {
        let entity = engine.get_entity_mut(soldier).unwrap();
        let actor = entity.actor_data_mut().unwrap();
        actor.installed_order = Some(InstalledActorOrder {
            order_id: old_order_id,
            order_type: OrderType::WaitingSword,
        });
        actor.retained_waiting_sword_order_id = Some(old_order_id);
        entity.sprite_mut().last_processed_order_id = old_order_id.get();
        entity.sprite_mut().frame_count = 5;
    }

    let mut wait = SequenceElement::new_generic(1, Command::Wait, Some(soldier));
    wait.orders
        .push_back(Order::new(OrderType::WaitingSword, 0.0, 0.0, new_order_id));
    let sequence = engine.orders.sequence_manager.launch_element(wait);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.publish_selected_order_as_installed(soldier);

    let entity = engine.get_entity(soldier).unwrap();
    let actor = entity.actor_data().unwrap();
    assert_eq!(actor.installed_order.unwrap().order_id, new_order_id);
    assert_eq!(actor.retained_waiting_sword_order_id, None);
    assert_eq!(actor.last_execute_order_id, Some(new_order_id));
    assert_eq!(entity.sprite().last_processed_order_id, new_order_id.get());
    assert_eq!(entity.sprite().frame_count, 5);
}

#[test]
fn exhausted_generic_order_carrier_terminates_on_resume() {
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    let mut element = SequenceElement::new_generic(1, Command::Generic, Some(soldier));
    element.posture_after_transition = Posture::Upright;
    let sequence = engine.orders.sequence_manager.launch_element(element);

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("terminated carrier remains until cleanup")
            .state,
        SequenceState::Terminated
    );
}

#[test]
fn accepted_zero_order_damage_preserves_in_progress_motion_edge() {
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(soldier)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .continuation
        .motion_state = MotionState::Done;

    // A malformed damage element is still accepted by Actor::Instruct, then
    // its command translation terminates it synchronously without an order.
    // Original writes IN_PROGRESS between those two events.
    let mut damage = SequenceElement::new_generic(1, Command::ReceiveSwordDamage, Some(soldier));
    damage.posture_after_transition = Posture::Upright;
    let sequence = engine.orders.sequence_manager.launch_element(damage);

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("terminated damage element remains until cleanup")
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(soldier)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::InProgress,
        "accepted Actor::Instruct must expose its motion edge even when translation terminates"
    );
}

#[test]
fn assert_position_translation_preserves_terminal_motion_edge() {
    use crate::coordinates::MapPoint;
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceElementData, SequenceState};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    let civilian = engine.add_entity(make_test_civilian(Posture::Upright));
    engine
        .get_entity_mut(civilian)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .continuation
        .motion_state = MotionState::Terminated;

    let mut assertion = SequenceElement::new_movement(
        1,
        Command::AssertPosition,
        Some(civilian),
        OrderType::WalkingUpright,
    );
    assertion.posture_after_transition = Posture::Upright;
    if let SequenceElementData::Movement {
        destination,
        tolerance,
        ..
    } = &mut assertion.data
    {
        *destination = MapPoint::ZERO;
        *tolerance = 10.0;
    }
    let sequence = engine.orders.sequence_manager.launch_element(assertion);

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("position assertion remains inspectable")
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(civilian)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::Terminated,
        "Translate-time SetState must skip Actor::Instruct's IN_PROGRESS epilogue"
    );
}

#[test]
fn entity_phase_completion_resumes_postponed_work_in_same_manager_drain() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));

    let mut blocker = SequenceElement::new_generic(1, Command::Generic, Some(owner));
    blocker.priority = SequencePriority::PostponeEverythingButInjuries;
    blocker.posture_after_transition = Posture::Upright;
    blocker.orders.push_back(Order::new(
        OrderType::TransitionRaisingSword,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    let blocker_sequence = engine.orders.sequence_manager.launch_element(blocker);
    // This fixture starts after instruction, at the actor-execution boundary.
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .element_in_progress(blocker_sequence, 0);

    let mut successor = SequenceElement::new_generic(1, Command::Generic, Some(owner));
    successor.priority = SequencePriority::Normal;
    successor.posture_after_transition = Posture::Upright;
    successor.orders.push_back(Order::new(
        OrderType::RunningWithSword,
        10.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    let successor_sequence = engine.orders.sequence_manager.launch_element(successor);
    // Consume the original launch registration: arbitration has postponed
    // this work behind the live blocker, so only blocker completion may
    // register it again.
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .postpone_element(successor_sequence, 0);
    engine
        .orders
        .sequence_manager
        .get_element_mut(blocker_sequence, 0)
        .unwrap()
        .cross_postponed = Some((successor_sequence, 0));

    // Actor execution ends before SequenceManager::Hourglass. The terminal
    // card is intentionally still pending when the sequence phase begins.
    engine
        .orders
        .sequence_manager
        .element_terminated(blocker_sequence, 0);

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(successor_sequence, 0)
            .unwrap()
            .state,
        SequenceState::InProgress,
        "postponed work released by actor completion must be instructed by the same manager drain"
    );
}

#[test]
fn postponing_pathfinding_movement_restores_move_and_cancels_failure() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut movement = SequenceElement::new_movement(
        1,
        Command::MoveWaiting,
        Some(owner),
        OrderType::WalkingUpright,
    );
    movement.priority = SequencePriority::Normal;
    movement.orders.push_back(Order::new(
        OrderType::Freezing,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);
    engine.orders.failed_path_requests.push(
        crate::engine::movement::FailedPathRequest::from_pending(
            crate::engine::movement::PendingPathRequest::test_request(owner, movement_sequence, 0),
            0,
        ),
    );

    let mut blocker = SequenceElement::new(1, Command::LeaveAttentiveMode, Some(owner));
    blocker.priority = SequencePriority::PostponeEverythingButInjuries;
    let blocker_sequence = engine.orders.sequence_manager.launch_element(blocker);

    engine.engine_postpone(blocker_sequence, 0, movement_sequence, 0);

    let movement = engine
        .orders
        .sequence_manager
        .get_element(movement_sequence, 0)
        .expect("postponed movement remains registered");
    assert_eq!(movement.state, SequenceState::Postponed);
    assert_eq!(
        movement.command,
        Command::Move,
        "postponed MoveWaiting must be translated again when it resumes"
    );
    assert!(
        movement.orders.is_empty(),
        "postponed movement must discard its pathfinder freezing order"
    );
    assert!(
        engine.orders.failed_path_requests.is_empty(),
        "postponing MoveWaiting must cancel its pathfinder failure bookkeeping"
    );
}

#[test]
fn postponing_resolved_movement_restores_untranslated_move() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut movement =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::WalkingUpright);
    movement.priority = SequencePriority::Normal;
    movement.orders.push_back(Order::new(
        OrderType::WalkingUpright,
        100.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);

    let mut blocker = SequenceElement::new(1, Command::LeaveAttentiveMode, Some(owner));
    blocker.priority = SequencePriority::PostponeEverythingButInjuries;
    let blocker_sequence = engine.orders.sequence_manager.launch_element(blocker);

    engine.engine_postpone(blocker_sequence, 0, movement_sequence, 0);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(movement_sequence, 0)
            .expect("postponed movement remains registered")
            .command,
        Command::Move,
        "postponed MoveOk must discard its translated path and translate again on resume"
    );
}

#[test]
fn post_seek_handoff_clears_selected_movement_goal() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{Sequence, SequenceElement};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .position_iface_mut()
        .set_map_goal(crate::coordinates::MapPoint::new(70.0, 80.0));

    let mut post_seek = Sequence::new();
    post_seek.append_element(SequenceElement::new_generic(1, Command::Wait, Some(owner)));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .post_seek_sequence = Some(Box::new(post_seek));

    let seek =
        SequenceElement::new_movement(1, Command::Seek, Some(owner), OrderType::WalkingUpright);
    let seek_sequence = engine.orders.sequence_manager.launch_element(seek);
    engine
        .orders
        .sequence_manager
        .element_in_progress(seek_sequence, 0);

    assert!(engine.start_post_seek_sequence(owner, Some((seek_sequence, 0))));
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        crate::coordinates::MapPoint::ZERO,
        "SendCondolationCard clears the selected seek goal before post-seek launch"
    );
}

#[test]
fn initial_seek_dispatch_clears_outgoing_movement_goal_until_first_execute() {
    use crate::element::{Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequenceElementData, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let stale_goal = MapPoint::new(70.0, 80.0);

    let mut outgoing =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::RunningUpright);
    outgoing.priority = SequencePriority::Normal;
    outgoing.posture_after_transition = Posture::Upright;
    outgoing.orders.push_back(Order::new(
        OrderType::RunningUpright,
        stale_goal.x,
        stale_goal.y,
        engine.orders.allocate_order_id(),
    ));
    let outgoing_sequence = engine.orders.sequence_manager.launch_element(outgoing);
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .element_in_progress(outgoing_sequence, 0);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.actor_data_mut().unwrap().active_movement =
            ActiveMovement::new(outgoing_sequence, 0);
        let position = entity.position_iface_mut();
        position.set_move_box(crate::coordinates::MoveBox::from_coords(
            -5.0, -5.0, 5.0, 5.0,
        ));
        position.set_map_goal(stale_goal);
    }

    let new_goal = MapPoint::new(100.0, 0.0);
    let mut seek =
        SequenceElement::new_movement(1, Command::Seek, Some(owner), OrderType::WalkingUpright);
    seek.priority = SequencePriority::Normal;
    if let SequenceElementData::Movement {
        destination, flags, ..
    } = &mut seek.data
    {
        *destination = new_goal;
        *flags |= crate::sequence::MoveFlags::SEEK;
    } else {
        unreachable!("new_movement must produce movement data");
    }
    let seek_sequence = engine.orders.sequence_manager.launch_element(seek);

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        MapPoint::ZERO,
        "Original interrupts its selected transient Seek before launching the concrete movement"
    );
    let transient_seek = engine
        .orders
        .sequence_manager
        .get_element(seek_sequence, 0)
        .expect("transient Seek wrapper remains inspectable");
    assert_eq!(transient_seek.state, SequenceState::Interrupted);

    let concrete_sequence = crate::sequence::SequenceId(seek_sequence.0 + 1);
    let concrete_seek = engine
        .orders
        .sequence_manager
        .get_element(concrete_sequence, 0)
        .expect("concrete seek movement should be launched separately");
    assert_eq!(concrete_seek.state, SequenceState::InProgress);
    assert_eq!(concrete_seek.command, Command::MoveOk);
    assert!(
        concrete_seek.current_order().is_some(),
        "the concrete movement is prepared, but its first Execute must install the new sprite goal"
    );
}

#[test]
fn parry_sword_queues_transition_and_hold_orders() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(soldier)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::WaitingSword;

    let seq_id =
        engine
            .orders
            .sequence_manager
            .launch_element(crate::sequence::SequenceElement::new(
                1,
                Command::ParrySword,
                Some(soldier),
            ));
    engine.dispatch_parry_sword(soldier, false, seq_id, 0);

    let elem = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("parry element should remain live");
    assert_eq!(elem.state, crate::sequence::SequenceState::InProgress);
    assert_eq!(
        elem.orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![
            OrderType::TransitionWaitingSwordParryingSword,
            OrderType::ParryingSword,
        ]
    );
}

#[test]
fn parry_sword_terminates_when_either_parry_is_already_active() {
    use crate::element::{ActionState, Command, Posture};

    for (action_state, command, low) in [
        (ActionState::ParryingSword, Command::ParrySword, false),
        (ActionState::ParryingSwordLow, Command::ParrySword, false),
        (ActionState::ParryingSword, Command::ParrySwordLow, true),
        (ActionState::ParryingSwordLow, Command::ParrySwordLow, true),
    ] {
        let mut engine = EngineInner::new();
        let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
        engine
            .get_entity_mut(soldier)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .action_state = action_state;

        let seq_id =
            engine
                .orders
                .sequence_manager
                .launch_element(crate::sequence::SequenceElement::new(
                    1,
                    command,
                    Some(soldier),
                ));
        engine.dispatch_parry_sword(soldier, low, seq_id, 0);

        let elem = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .expect("parry element remains available for its condolence");
        assert_eq!(
            elem.state,
            crate::sequence::SequenceState::Terminated,
            "{command:?} must terminate from {action_state:?}"
        );
        assert!(
            elem.orders.is_empty(),
            "an already-active parade must not receive another hold order"
        );
    }
}

#[test]
fn stop_parry_sword_queues_exit_transition() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(soldier)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::ParryingSword;

    let seq_id =
        engine
            .orders
            .sequence_manager
            .launch_element(crate::sequence::SequenceElement::new(
                1,
                Command::StopParrySword,
                Some(soldier),
            ));
    engine.dispatch_stop_parry(soldier, seq_id, 0);

    let elem = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("stop-parry element should remain live");
    assert_eq!(elem.state, crate::sequence::SequenceState::InProgress);
    assert_eq!(
        elem.orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![OrderType::TransitionParryingSwordWaitingSword]
    );
}

/// A LeaningOut soldier that receives a command requiring Upright
/// (e.g. `Move`) must snap to Upright and queue the
/// `TransitionLeaningOutWaitingAlerted` animation so the lean-out-
/// window unstick transition plays.
#[test]
fn soldier_leaning_out_to_upright_on_move() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::LeaningOut));

    let changed = engine.auto_leave_disguise_if_needed(soldier_id, Command::Move);
    assert!(changed, "auto-leave should fire for LeaningOut + Move");

    let entity = engine.get_entity(soldier_id).expect("soldier present");
    assert_eq!(
        entity.element_data().posture,
        Posture::Upright,
        "posture should snap to Upright"
    );

    let next_order = engine
        .orders
        .sequence_manager
        .current_order_for_actor(soldier_id)
        .map(|(_, _, o)| o.order_type);
    assert_eq!(
        next_order,
        Some(OrderType::TransitionLeaningOutWaitingAlerted),
        "lean-out transition animation should be queued"
    );
}

/// An Upright soldier invoked with a posture-neutral command should
/// not be touched by `auto_leave_disguise_if_needed`.
#[test]
fn soldier_upright_move_skips_auto_leave() {
    use crate::element::{Command, Posture};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Upright));

    let changed = engine.auto_leave_disguise_if_needed(soldier_id, Command::Move);
    assert!(!changed, "no transition needed for an Upright soldier");

    let entity = engine.get_entity(soldier_id).expect("soldier present");
    assert_eq!(entity.element_data().posture, Posture::Upright);
    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(soldier_id)
            .is_none(),
        "no animation should be queued"
    );
}

#[test]
fn fresh_wait_replaces_pre_init_upright_idle_with_authored_sitting_idle() {
    use crate::element::{ActionState, Posture};
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    let mut display = HostDisplayState::default();
    let assets = LevelAssets::default();

    // Mission/script initialization can make the actor execute an upright
    // wait before AI InitState evaluates its authored initial animation.
    engine.actor_wait(owner);
    engine.hourglass_phase_sequences(&crate::sim_rng::test_context(), &mut display, &assets);

    {
        let actor = engine.get_entity_mut(owner).expect("soldier present");
        actor.set_posture(Posture::Sitting);
        actor.actor_data_mut().expect("actor data").action_state = ActionState::Waiting;
    }

    // RHArtificialIntelligence::InitState calls Wait again after SetStates.
    engine.actor_wait(owner);
    engine.hourglass_phase_sequences(&crate::sim_rng::test_context(), &mut display, &assets);

    let order = engine
        .orders
        .sequence_manager
        .current_order_for_actor(owner)
        .map(|(_, _, order)| order.order_type);
    assert_eq!(order, Some(OrderType::Sitting));
    assert_eq!(
        engine
            .get_entity(owner)
            .expect("soldier present")
            .element_data()
            .posture,
        Posture::Sitting
    );
}

#[test]
fn idle_wait_runs_while_future_owner_action_is_behind_ownerless_timer() {
    use crate::element::{Command, Posture};
    use crate::sequence::{
        Field, FieldValue, Sequence, SequenceElement, SequencePriority, SequenceState,
    };

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    // The officer-conversation regression leaves the actor idle after its
    // Turn while a later owned PlayAnim waits behind an ownerless Timer.
    // Original Actor::Hourglass sees no current order and launches its
    // low-priority Wait; future command levels do not suppress that idle.
    let mut sequence = Sequence::new();
    let mut timer = SequenceElement::new_generic(1, Command::Timer, None);
    timer.set_property(Field::Timer, FieldValue::Integer(50));
    sequence.append_element(timer);
    sequence.append_element(SequenceElement::new(2, Command::PlayAnim, Some(owner)));
    let scripted_sequence = engine.orders.sequence_manager.launch_sequence(sequence);

    let future = engine
        .orders
        .sequence_manager
        .get_element(scripted_sequence, 1)
        .expect("future actor action exists");
    assert_eq!(future.state, SequenceState::Todo);
    assert!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
            .is_none(),
        "a future command level is not the actor's current order"
    );

    engine.ensure_wait_element(owner);
    let mut display = HostDisplayState::default();
    let assets = LevelAssets::default();
    engine.hourglass_phase_sequences(&crate::sim_rng::test_context(), &mut display, &assets);

    let (wait_sequence, wait_index) = engine
        .orders
        .sequence_manager
        .current_element_for_actor(owner)
        .expect("idle actor must execute a default Wait");
    let wait = engine
        .orders
        .sequence_manager
        .get_element(wait_sequence, wait_index)
        .expect("default Wait remains live");
    assert_eq!(wait.command, Command::Wait);
    assert_eq!(wait.priority, SequencePriority::Wait);
    assert_eq!(wait.state, SequenceState::InProgress);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(scripted_sequence, 1)
            .expect("future actor action remains queued")
            .state,
        SequenceState::Todo
    );
}

#[test]
fn play_anim_uses_custom_wrapper_instead_of_requested_animation_semantics() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{Field, FieldValue, SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(owner)
        .expect("test soldier exists")
        .actor_data_mut()
        .expect("test soldier is an actor")
        .action_state = ActionState::Bored;

    let mut element = SequenceElement::new_generic(1, Command::PlayAnim, Some(owner));
    element.set_property(
        Field::AnimationId,
        FieldValue::Animation(OrderType::Pointing),
    );
    let sequence = engine.orders.sequence_manager.launch_element(element);
    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("PlayAnim element remains live");
    assert_eq!(element.state, SequenceState::InProgress);
    assert_eq!(
        element.current_order().map(|order| order.order_type),
        Some(OrderType::PlayCustom),
        "Pointing is only the requested sprite animation; Original executes the PlayCustom wrapper"
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .expect("test soldier remains")
            .actor_data()
            .expect("test soldier remains an actor")
            .action_state,
        ActionState::Bored,
        "translating custom Pointing must not apply Pointing's Waiting state"
    );
}

/// An attentive-mode transition on an idle soldier queues
/// `TransitionWaitingUprightWaitingAlerted` as an order on the
/// sequence element.
#[test]
fn soldier_enter_attentive_mode_queues_transition_anim() {
    let mut display = HostDisplayState::default();
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Upright));

    // Launch the EnterAttentiveMode element first; `ensure_wait_element`
    // is a no-op once another live element exists for the actor.  This
    // matches level-load ordering: spawn → (maybe scripted elements) →
    // ensure_wait_element covers only the actors left idle.
    // Stamp `posture_after_transition = Upright` at launch.
    let mut elem = SequenceElement::new(1, Command::EnterAttentiveMode, Some(soldier_id));
    elem.posture_after_transition = Posture::Upright;
    engine.launch_element(elem);
    engine.ensure_wait_element(soldier_id);

    let mut assets = LevelAssets::default();
    let mut dev = crate::engine::DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let active = engine
        .orders
        .sequence_manager
        .current_order_for_actor(soldier_id)
        .map(|(_, _, o)| o.order_type);
    assert_eq!(
        active,
        Some(OrderType::TransitionWaitingUprightWaitingAlerted),
        "the transition order should be the front of the actor's current element"
    );
}

/// Regression: calling `set_soldier_attentive_mode` on an Upright
/// soldier (the path real game code hits via `pending_set_attentive_mode`
/// when an enemy spots the PC) must queue the alerted-transition
/// animation.  The previous bug left
/// `SequenceElement::posture_after_transition` at `Posture::Undefined`
/// because only `ensure_wait_element` and `auto_leave_disguise_if_needed`
/// stamped it; `arbitrate_instruct` now stamps it unconditionally
/// (`set_posture_after_transition(get_posture())`).
#[test]
fn set_soldier_attentive_mode_plays_transition_from_upright() {
    let mut display = HostDisplayState::default();
    use crate::element::Posture;
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Upright));

    // Drive the engine-side helper the way the AI does it — no explicit
    // posture stamping; arbitrate_instruct must supply it.  Launch the
    // attentive element before `ensure_wait_element` so the latter
    // no-ops (matching the AI drain ordering in `tick_enemy_ai` where
    // `set_soldier_attentive_mode` fires from the per-NPC pending drain
    // and only actors left idle get a Wait element).
    engine.set_soldier_attentive_mode(soldier_id, true, false);
    engine.ensure_wait_element(soldier_id);

    let mut assets = LevelAssets::default();
    let mut dev = crate::engine::DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let active = engine
        .orders
        .sequence_manager
        .current_order_for_actor(soldier_id)
        .map(|(_, _, o)| o.order_type);
    assert_eq!(
        active,
        Some(OrderType::TransitionWaitingUprightWaitingAlerted),
        "transition-to-alerted animation should be the actor's current order"
    );
}

#[test]
fn set_soldier_attentive_mode_plays_transition_while_movement_is_postponed() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut movement = SequenceElement::new_movement(
        1,
        Command::MoveOk,
        Some(soldier_id),
        OrderType::RunningUpright,
    );
    movement.priority = SequencePriority::Normal;
    movement.orders.push_back(Order::new(
        OrderType::RunningUpright,
        100.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);

    engine.set_soldier_attentive_mode(soldier_id, true, false);

    // The attentive element is only registered with the manager here; its
    // Instruct (and the movement postpone it causes) runs at the next
    // manager hourglass, matching the deferred launch semantics.
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(movement_sequence, 0)
            .expect("movement remains registered")
            .state,
        SequenceState::InProgress
    );

    let mut display = HostDisplayState::default();
    let mut assets = LevelAssets::default();
    let mut dev = crate::engine::DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let movement = engine
        .orders
        .sequence_manager
        .get_element(movement_sequence, 0)
        .expect("postponed movement remains registered");
    assert_eq!(movement.state, SequenceState::Postponed);
    assert_eq!(movement.command, Command::Move);
    assert!(movement.orders.is_empty());

    // The attentive element's transition generation must first stop the
    // running actor (its action state exits MOVING before entering the
    // alerted stance), so the stop transition fronts the order queue with
    // the alerted transition queued behind it.
    let (attentive_seq, attentive_idx, front) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(soldier_id)
        .expect("attentive element should be current after the postpone");
    assert_eq!(
        front.order_type,
        OrderType::TransitionRunningUprightWaitingUpright
    );
    let attentive_orders: Vec<OrderType> = engine
        .orders
        .sequence_manager
        .get_element(attentive_seq, attentive_idx)
        .expect("attentive element remains registered")
        .orders
        .iter()
        .map(|order| order.order_type)
        .collect();
    assert!(
        attentive_orders.contains(&OrderType::TransitionWaitingUprightWaitingAlerted),
        "postponing a movement must not suppress the attentive transition, got {attentive_orders:?}",
    );
}

#[test]
fn arbitration_ignores_serialized_order_ai_lock_like_original() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut current = SequenceElement::new(1, Command::Move, Some(owner));
    current.priority = SequencePriority::Normal;
    current
        .orders
        .push_back(Order::test_new(OrderType::WalkingUpright, 10.0, 0.0));
    current
        .orders
        .push_back(Order::test_new(OrderType::WalkingUpright, 20.0, 0.0));
    current.orders.front_mut().unwrap().lock_ai = true;
    let current_seq = engine.orders.sequence_manager.launch_element(current);
    engine
        .orders
        .sequence_manager
        .element_in_progress(current_seq, 0);

    let mut incoming = SequenceElement::new(1, Command::Turn, Some(owner));
    incoming.priority = SequencePriority::Preference;
    let incoming_seq = engine.orders.sequence_manager.launch_element(incoming);

    let accepted = engine.arbitrate_instruct(incoming_seq, 0);
    assert!(
        accepted,
        "Original CanInterruptNow always accepts a live current order"
    );

    let current = engine
        .orders
        .sequence_manager
        .get_element(current_seq, 0)
        .unwrap();
    assert_eq!(current.state, SequenceState::Postponed);
    assert!(
        current.orders.is_empty(),
        "Original postponement discards the translated current order chain"
    );

    let incoming = engine
        .orders
        .sequence_manager
        .get_element(incoming_seq, 0)
        .unwrap();
    assert_eq!(incoming.state, SequenceState::Todo);
    assert_eq!(incoming.cross_postponed, Some((current_seq, 0)));
}

#[test]
fn duplicate_instruct_does_not_arbitrate_an_element_against_itself() {
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    let mut element = SequenceElement::new(1, Command::Move, Some(owner));
    element.priority = SequencePriority::Normal;
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    assert!(engine.arbitrate_instruct(sequence, 0));
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );
}

#[test]
fn interrupt_callback_arbitrates_nested_work_against_incoming_selection() {
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut outgoing = SequenceElement::new(1, Command::SwordstrikeSmalltalkLeft, Some(owner));
    outgoing.priority = SequencePriority::Wait;
    // Interrupt arbitration requires the in-progress element to carry its
    // current order, mirroring the assertion in the original manager.
    outgoing.orders.push_back(crate::order::Order::test_new(
        crate::order::OrderType::WaitingUpright,
        0.0,
        0.0,
    ));
    let outgoing_sequence = engine.orders.sequence_manager.launch_element(outgoing);
    engine
        .orders
        .sequence_manager
        .element_in_progress(outgoing_sequence, 0);

    let mut incoming = SequenceElement::new(1, Command::ReceiveSwordDamage, Some(owner));
    incoming.priority = SequencePriority::Injury;
    let incoming_sequence = engine.orders.sequence_manager.launch_element(incoming);
    assert!(engine.arbitrate_instruct(incoming_sequence, 0));

    engine
        .orders
        .sequence_manager
        .begin_instruct_callback(owner, incoming_sequence, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner),
        Some((incoming_sequence, 0)),
        "the outgoing SetState callback must observe incoming injury as selected"
    );

    let mut nested = SequenceElement::new(1, Command::Turn, Some(owner));
    nested.priority = SequencePriority::Normal;
    let nested_sequence = engine.orders.sequence_manager.launch_element(nested);
    assert!(
        !engine.arbitrate_instruct(nested_sequence, 0),
        "recursive normal work must arbitrate against the selected injury"
    );
    assert_ne!(
        engine
            .orders
            .sequence_manager
            .get_element(nested_sequence, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );

    engine
        .orders
        .sequence_manager
        .end_instruct_callback(owner, incoming_sequence, 0);
}

#[test]
fn done_propagation_requires_the_current_order_identity() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{CascadeFlags, SequenceElement};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let stale_order_id = engine.orders.allocate_order_id();
    let mut interrupted = SequenceElement::new_generic(1, Command::Generic, Some(owner));
    interrupted.orders.push_back(Order::new(
        OrderType::WaitingUpright,
        0.0,
        0.0,
        stale_order_id,
    ));
    let interrupted_sequence = engine.orders.sequence_manager.launch_element(interrupted);
    engine
        .orders
        .sequence_manager
        .element_in_progress(interrupted_sequence, 0);
    engine.orders.sequence_manager.element_interrupted(
        interrupted_sequence,
        0,
        CascadeFlags::empty(),
    );

    let replacement_order_id = engine.orders.allocate_order_id();
    let mut replacement = SequenceElement::new_generic(1, Command::Generic, Some(owner));
    replacement.orders.push_back(Order::new(
        OrderType::WaitingAlerted,
        0.0,
        0.0,
        replacement_order_id,
    ));
    let replacement_sequence = engine.orders.sequence_manager.launch_element(replacement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(replacement_sequence, 0);

    {
        let sprite = &mut engine
            .get_entity_mut(owner)
            .unwrap()
            .element_data_mut()
            .sprite;
        sprite.last_motion_state = Some(MotionState::Done);
        sprite.last_processed_order_id = stale_order_id.get();
    }
    engine.propagate_done_to_current_orders();

    assert!(
        !engine
            .orders
            .sequence_manager
            .get_element(replacement_sequence, 0)
            .unwrap()
            .current_order()
            .unwrap()
            .done,
        "a stale Done reported by the interrupted order must not complete its replacement"
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .element_data()
            .sprite
            .last_motion_state,
        None,
        "the stale transient result must still be consumed"
    );

    {
        let sprite = &mut engine
            .get_entity_mut(owner)
            .unwrap()
            .element_data_mut()
            .sprite;
        sprite.last_motion_state = Some(MotionState::Done);
        sprite.last_processed_order_id = replacement_order_id.get();
    }
    engine.propagate_done_to_current_orders();

    assert!(
        engine
            .orders
            .sequence_manager
            .get_element(replacement_sequence, 0)
            .unwrap()
            .current_order()
            .unwrap()
            .done,
        "Done from the currently dispatched order ID must still propagate"
    );
}

#[test]
fn pc_shoot_bow_waits_through_load_and_wait_then_retries_only_while_aiming() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let sim = crate::sim_rng::test_context();
    let assets = crate::engine::LevelAssets::new();
    let mut engine = EngineInner::new();
    let pc = engine.add_entity(make_test_pc(Posture::Upright));
    engine
        .get_entity_mut(pc)
        .unwrap()
        .element_data_mut()
        .sprite
        .last_action = OrderType::TransitionLoadingBow;

    // A missing antagonist makes the eventual Translate deterministic and
    // side-effect free; this regression is about Human::Instruct admission,
    // not projectile construction.
    let incoming = SequenceElement::new_interaction(1, Command::ShootBow, Some(pc), None);
    let incoming_seq = engine.launch_element_for_owner(&sim, &assets, incoming);

    let held = engine
        .orders
        .sequence_manager
        .get_element(incoming_seq, 0)
        .unwrap();
    assert_eq!(held.state, SequenceState::Todo);
    assert_eq!(held.priority, SequencePriority::NotYetSet);
    assert_eq!(held.posture_after_transition, Posture::Undefined);
    assert_eq!(held.action_state_after_transition, ActionState::Waiting);
    assert!(held.orders.is_empty());
    assert_eq!(held.cross_postponed, None);
    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .human_data()
            .unwrap()
            .pending_shoots,
        [crate::sequence::SequenceElementRef::new(incoming_seq, 0)]
    );

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::default();

    // Loading has reported DONE, but the sprite still names the completed
    // loading animation. The following Wait frame is not sufficient either.
    engine.process_shoot_list_for(&sim, &assets, pc);
    engine
        .get_entity_mut(pc)
        .unwrap()
        .element_data_mut()
        .sprite
        .last_action = OrderType::WaitingUpright;
    engine.process_shoot_list_for(&sim, &assets, pc);
    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .human_data()
            .unwrap()
            .pending_shoots
            .len(),
        1
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(incoming_seq, 0)
            .unwrap()
            .priority,
        SequencePriority::NotYetSet
    );

    // Only the bow-aiming idle admits the retained element. Instruct then
    // reaches Translate and consumes the FIFO entry (the deliberately absent
    // target makes this particular element Impossible).
    engine
        .get_entity_mut(pc)
        .unwrap()
        .element_data_mut()
        .sprite
        .last_action = OrderType::AimingWithBow;
    engine.process_shoot_list_for(&sim, &assets, pc);
    assert!(
        engine
            .get_entity(pc)
            .unwrap()
            .human_data()
            .unwrap()
            .pending_shoots
            .is_empty()
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(incoming_seq, 0)
            .unwrap()
            .state,
        SequenceState::Impossible
    );
}

#[test]
fn started_pass_door_rejects_new_move() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let sim = crate::sim_rng::test_context();
    let assets = crate::engine::LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut current_pass =
        SequenceElement::new_movement(1, Command::PassDoor, Some(owner), OrderType::WalkingUpright);
    current_pass.priority = SequencePriority::NonInterruptable;
    let pass_seq = engine.orders.sequence_manager.launch_element(current_pass);
    engine
        .orders
        .sequence_manager
        .element_in_progress(pass_seq, 0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sequence_element_started = true;

    let incoming =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    let incoming_seq = engine.launch_element_for_owner(&sim, &assets, incoming);

    let pass = engine
        .orders
        .sequence_manager
        .get_element(pass_seq, 0)
        .unwrap();
    assert_eq!(pass.state, SequenceState::InProgress);

    let incoming = engine
        .orders
        .sequence_manager
        .get_element(incoming_seq, 0)
        .unwrap();
    assert_eq!(incoming.state, SequenceState::Impossible);
}

#[test]
fn executing_pass_door_postpones_new_move() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let sim = crate::sim_rng::test_context();
    let assets = crate::engine::LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut current_pass =
        SequenceElement::new_movement(1, Command::PassDoor, Some(owner), OrderType::WalkingUpright);
    current_pass.priority = SequencePriority::NonInterruptable;
    let pass_seq = engine.orders.sequence_manager.launch_element(current_pass);
    engine
        .orders
        .sequence_manager
        .element_in_progress(pass_seq, 0);
    // Execute clears this flag at the end of the actor's frame.
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sequence_element_started = false;

    let incoming =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    let incoming_seq = engine.launch_element_for_owner(&sim, &assets, incoming);

    let pass = engine
        .orders
        .sequence_manager
        .get_element(pass_seq, 0)
        .unwrap();
    assert_eq!(pass.state, SequenceState::InProgress);
    assert_eq!(pass.cross_postponed, Some((incoming_seq, 0)));

    let incoming = engine
        .orders
        .sequence_manager
        .get_element(incoming_seq, 0)
        .unwrap();
    assert_eq!(incoming.state, SequenceState::Postponed);
}

/// A Crouched soldier receiving `ENTER_ATTENTIVE_MODE` must first
/// auto-stand (CROUCH_UP) before the alerted transition can play,
/// because `get_transition_flags_soldier` for this command sets
/// `CHANGEPOSTURE_MUST_BE_UPRIGHT` without `CAN_BE_CROUCHED`.
/// Posture transition generation auto-inserts a `CROUCH_UP` translate and flips the element's
/// `posture_after_transition` to Upright; the soldier's own Translate
/// then queues the transition animation on the now-Upright element.
///
/// The "Consider as done" else-branch at
/// the soldier command only fires when GenerateTransition couldn't promote
/// posture to Upright (e.g. on a ladder).  That arm
/// isn't reachable from Crouched once GenerateTransition is wired in.
#[test]
fn soldier_enter_attentive_mode_from_crouched_stands_first() {
    let mut display = HostDisplayState::default();
    use crate::element::{Command, Posture};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Crouched));

    // Leave `posture_after_transition` undefined: the deferred Instruct
    // stamps it from the actor's live (Crouched) posture, and transition
    // generation then promotes it to Upright via the crouch-up animation.
    // A pre-stamped posture would skip that transition pass entirely.
    let elem = SequenceElement::new(1, Command::EnterAttentiveMode, Some(soldier_id));
    engine.launch_element(elem);
    engine.ensure_wait_element(soldier_id);

    let mut assets = LevelAssets::default();
    let mut dev = crate::engine::DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    // `MakePostureTransition` translates the CROUCH_UP then the element's
    // `posture_after_transition` is Upright; the ENTER_ATTENTIVE_MODE
    // Translate queues the alerted transition animation on top.  The
    // actor's current order is whatever sits at the front of the order
    // queue — the crouch-up animation runs first.
    let front = engine
        .orders
        .sequence_manager
        .current_order_for_actor(soldier_id)
        .map(|(_, _, o)| o.order_type);
    assert_eq!(
        front,
        Some(crate::order::OrderType::TransitionCrouchingUp),
        "crouch-up transition animation should play first"
    );
}

// ─── Waypoint-script VM dispatch ───────────────────────────────────
//
// Covers the per-waypoint VM wiring added to `MissionScript`:
// `bind_waypoint` + the shared ScriptVmKey driver. Each scripted waypoint
// carries its own VM and `Initialize()` + `ReachPoint(actor)` dispatch
// into that VM.

/// Build a minimal SCB with one class `TestWaypoint` that exposes
/// empty `Initialize` and `ReachPoint` functions (body: just
/// `BeginFunction` + `Return`).  Returns the parsed `ScbFile` shaped
/// for `MissionScript::from_scb`.
fn scripted_waypoint_scb() -> crate::scb::ScbFile {
    use crate::scb::{ClassEntry, Function, ScbFile};
    use crate::vm::{Opcode, Quad};

    let begin = Quad {
        operation: Opcode::BeginFunction as u8,
        operands: [0; 8],
    };
    let ret = Quad {
        operation: Opcode::Return as u8,
        operands: [0; 8],
    };

    let waypoint_class = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "TestWaypoint".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![
            Function {
                name: "Initialize".into(),
                address: 0,
                num_parameters: 0,
                size_of_return_value: 0,
                size_of_parameters: 0,
                size_of_volatile: 0,
                size_of_temporary: 0,
            },
            Function {
                name: "ReachPoint".into(),
                address: 2,
                num_parameters: 1,
                size_of_return_value: 0,
                size_of_parameters: 0,
                size_of_volatile: 0,
                size_of_temporary: 0,
            },
        ],
        quads: vec![begin, ret, begin, ret],
    };
    // `MissionScript::from_scb` requires a `StartUp` class to bind the
    // global instance against. Supply a stub so `from_scb` succeeds.
    let startup = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: Vec::new(),
        quads: Vec::new(),
    };

    ScbFile {
        version: crate::scb::SCB_VERSION,
        classes: vec![startup, waypoint_class],
    }
}

/// `bind_waypoint` inserts a `ScriptInstance` keyed by `(path, wp)`
/// without running callbacks through a bypass path.
#[test]
fn bind_waypoint_inserts_instance() {
    let scb = scripted_waypoint_scb();
    let mut script = MissionScript::from_scb(scb).expect("from_scb");
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut entity_store = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut entity_store,
        &mut ai_global,
        &mut fast_grid,
    );

    assert!(script.bind_waypoint(
        crate::ai::PathId::new(2).unwrap(),
        3,
        "TestWaypoint",
        &mut script_domains,
        &capabilities,
    ));
    assert!(
        script
            .waypoint_instances
            .contains_key(&(crate::ai::PathId::new(2).unwrap(), 3))
    );
}

#[test]
#[should_panic(expected = "Waypoint script class 'NonExistent'")]
fn bind_waypoint_rejects_missing_referenced_class() {
    let mut script = MissionScript::from_scb(scripted_waypoint_scb()).expect("from_scb");
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut entity_store = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut entity_store,
        &mut ai_global,
        &mut fast_grid,
    );
    script.bind_waypoint(
        crate::ai::PathId::new(4).unwrap(),
        0,
        "NonExistent",
        &mut script_domains,
        &capabilities,
    );
}

/// The Engine driver dispatches `ReachPoint(actor)` against the bound
/// waypoint instance and distinguishes a missing VM from a missing method.
#[test]
fn waypoint_driver_dispatches_and_distinguishes_missing_vm() {
    let scb = scripted_waypoint_scb();
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(MissionScript::from_scb(scb).expect("from_scb"));
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    engine
        .with_script_session(
            &crate::sim_rng::test_context(),
            &assets,
            |script, script_domains, capabilities| {
                assert!(script.bind_waypoint(
                    crate::ai::PathId::new(0).unwrap(),
                    0,
                    "TestWaypoint",
                    script_domains,
                    capabilities,
                ));
            },
        )
        .expect("mission installed");

    // Bound: call dispatches cleanly.
    let actor_handle = 42;
    let ret = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Waypoint(crate::ai::PathId::new(0).unwrap(), 0),
            "ReachPoint",
            &[actor_handle],
            crate::natives::ScriptCallFrame::default(),
        )
        .expect("ReachPoint");
    assert_eq!(ret, 0, "empty ReachPoint should return 0");

    // A missing required VM is structural, not an optional-method default.
    let missing = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Waypoint(crate::ai::PathId::new(7).unwrap(), 9),
            "ReachPoint",
            &[actor_handle],
            crate::natives::ScriptCallFrame::default(),
        )
        .expect_err("missing instance is an error");
    assert!(missing.contains("required VM is not bound"));

    // Missing function on a bound instance: also `Ok(0)`.
    let ret_no_fn = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Waypoint(crate::ai::PathId::new(0).unwrap(), 0),
            "NotAFunction",
            &[],
            crate::natives::ScriptCallFrame::default(),
        )
        .expect("missing function should be Ok(0)");
    assert_eq!(ret_no_fn, 0);
}

/// AI: `execute_waypoint_script(path, wp)` sets the pending dispatch
/// slot; the old unconditional `EventAfterScriptGoOn` fire-and-forget
/// behaviour was replaced by the engine-side drain.
#[test]
fn execute_waypoint_script_queues_pending_dispatch() {
    let mut ai = crate::ai::AiController::default();
    assert!(ai.outbox.reentrant.waypoint_script_reach_point.is_none());
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());

    let pid = crate::ai::PathId::new(5).unwrap();
    ai.execute_waypoint_script(pid, 2);

    assert_eq!(
        ai.outbox.reentrant.waypoint_script_reach_point,
        Some((pid, 2))
    );
    // AI must NOT pre-emptively queue `EventAfterScriptGoOn` — that
    // happens only after the engine dispatches `ReachPoint` and
    // confirms the script didn't transition into `DefaultScriptDriven`.
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());
}

/// `initialize_mission_script_with` walks the supplied hiking paths,
/// binds every `WaypointCommand::Script` waypoint, and runs
/// `Initialize()` on each.  Verifies the end-to-end level-load path
/// registers instances keyed by `(path_idx, wp_idx)`.
#[test]
fn initialize_mission_script_binds_waypoint_classes() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let scb = scripted_waypoint_scb();
    let mission_script = MissionScript::from_scb(scb).expect("from_scb");

    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    engine.scripts.mission = Some(mission_script);

    let paths = vec![
        RawHikingPath {
            waypoints: vec![
                RawWaypoint {
                    x: 0,
                    y: 0,
                    sector: 0,
                    level: 0,
                    command: WaypointCommand::None,
                },
                RawWaypoint {
                    x: 10,
                    y: 10,
                    sector: 0,
                    level: 0,
                    command: WaypointCommand::Script("TestWaypoint".into()),
                },
            ],
        },
        RawHikingPath {
            waypoints: vec![RawWaypoint {
                x: 20,
                y: 20,
                sector: 0,
                level: 0,
                command: WaypointCommand::Script("TestWaypoint".into()),
            }],
        },
    ];

    let assets = crate::engine::LevelAssets::new();
    engine.initialize_mission_script_with(sim, &assets, 0, &paths);

    let script = engine.scripts.mission.as_ref().expect("mission_script");
    // Two `Script` waypoints, both bound.
    assert!(
        script
            .waypoint_instances
            .contains_key(&(crate::ai::PathId::new(0).unwrap(), 1))
    );
    assert!(
        script
            .waypoint_instances
            .contains_key(&(crate::ai::PathId::new(1).unwrap(), 0))
    );
    // The `None`-command waypoint doesn't get a binding.
    assert!(
        !script
            .waypoint_instances
            .contains_key(&(crate::ai::PathId::new(0).unwrap(), 0))
    );
    assert_eq!(script.waypoint_instances.len(), 2);
}

/// Waypoint-script heaps round-trip through plain serde: heap bytes
/// written to the instance before serialising must come back
/// verbatim on deserialise.  This is the path `Engine::restore` uses
/// (via the full `EngineInner` serde derive), not a bespoke helper.
#[test]
fn waypoint_script_heap_round_trips_through_serde() {
    use crate::scb::{ClassEntry, Function, ScbFile};
    use crate::vm::{Opcode, Quad};

    // Class with a non-zero heap so we can poke distinct bytes in.
    let begin = Quad {
        operation: Opcode::BeginFunction as u8,
        operands: [0; 8],
    };
    let ret = Quad {
        operation: Opcode::Return as u8,
        operands: [0; 8],
    };
    let waypoint_class = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "HeapWaypoint".into(),
        size_of_member_variables: 8,
        member_variables: Vec::new(),
        functions: vec![Function {
            name: "Initialize".into(),
            address: 0,
            num_parameters: 0,
            size_of_return_value: 0,
            size_of_parameters: 0,
            size_of_volatile: 0,
            size_of_temporary: 0,
        }],
        quads: vec![begin, ret],
    };
    let startup = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: Vec::new(),
        quads: Vec::new(),
    };
    let scb = ScbFile {
        version: crate::scb::SCB_VERSION,
        classes: vec![startup, waypoint_class],
    };

    let mut script = MissionScript::from_scb(scb).expect("from_scb");
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut entity_store = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut entity_store,
        &mut ai_global,
        &mut fast_grid,
    );
    assert!(script.bind_waypoint(
        crate::ai::PathId::new(3).unwrap(),
        7,
        "HeapWaypoint",
        &mut script_domains,
        &capabilities,
    ));

    // Poke distinct bytes into the heap so a zero reset is detectable.
    script
        .waypoint_instances
        .get_mut(&(crate::ai::PathId::new(3).unwrap(), 7))
        .unwrap()
        .vm
        .heap
        .copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22]);

    // Serialise → deserialise → heap bytes must match.
    let json = serde_json::to_string(&script).expect("serialize");
    let restored: crate::engine::types::MissionScript =
        serde_json::from_str(&json).expect("deserialize");

    let inst = restored
        .waypoint_instances
        .get(&(crate::ai::PathId::new(3).unwrap(), 7))
        .expect("restored");
    assert_eq!(
        inst.vm.heap,
        &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22]
    );
}

/// Leaning-out soldiers that receive `Command::ShootBow` must keep
/// the lean-out pose — `GetTransitionFlags` pairs `MUST_BE_UPRIGHT`
/// with `CAN_BE_LEANING_OUT` for SHOOT_BOW, so the auto-leave should
/// skip.
#[test]
fn soldier_leaning_out_keeps_pose_for_shoot_bow() {
    use crate::element::{Command, Posture};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::LeaningOut));

    let changed = engine.auto_leave_disguise_if_needed(soldier_id, Command::ShootBow);
    assert!(
        !changed,
        "ShootBow + LeaningOut must stay in lean-out pose (CAN_BE_LEANING_OUT)"
    );

    let entity = engine.get_entity(soldier_id).expect("soldier present");
    assert_eq!(entity.element_data().posture, Posture::LeaningOut);
    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(soldier_id)
            .is_none(),
        "no unstick animation should be queued"
    );
}

/// The `auto_leave_disguise_if_needed` path should set
/// `posture_after_transition` and `action_state_after_transition`
/// on the in-flight sequence element.
#[test]
fn soldier_leaning_out_updates_sequence_element_fields() {
    use crate::element::{ActionState, Command, Posture};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::LeaningOut));

    // Launch a Move sequence element so there's an element to decorate.
    let elem = SequenceElement::new_movement(
        1,
        Command::Move,
        Some(soldier_id),
        crate::order::OrderType::WalkingUpright,
    );
    let seq_id = engine.launch_element(elem);

    let changed = engine.auto_leave_disguise_if_needed(soldier_id, Command::Move);
    assert!(changed);

    // Locate the element and verify the post-transition fields snap.
    let found = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .find(|s| s.id == seq_id)
        .and_then(|s| s.elements.iter().find(|e| e.command == Command::Move));
    let elem = found.expect("sequence element present");
    assert_eq!(elem.posture_after_transition, Posture::Upright);
    assert_eq!(elem.action_state_after_transition, ActionState::Waiting);
}

/// Regression: the synchronous `Instruct`-equivalent fires inside
/// `launch_element` for owned elements, so an element launched
/// mid-tick should be dispatched and reach `InProgress` during the
/// same `perform_hourglass` pass rather than idling one frame in
/// `Todo`.  The previous two-phase flow (launch → Todo → next-tick
/// arbitrate → dispatch) introduced a one-frame skew between launch
/// and visible state — `Instruct` runs synchronously inside
/// `LaunchSequenceElement` and ends with state `InProgress` after the
/// translate step inside the same call.
#[test]
fn launched_owned_element_reaches_in_progress_in_same_tick() {
    let mut display = HostDisplayState::default();
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Upright));

    // Launch a SitDown element — the NPC translate arm pushes a single
    // TransitionWaitingUprightSitting animation order onto it and flips
    // the element to InProgress inside the same hourglass pass.
    let elem = SequenceElement::new(1, Command::SitDown, Some(soldier_id));
    let seq_id = engine.launch_element(elem);
    engine.ensure_wait_element(soldier_id);

    let mut assets = LevelAssets::new();
    let mut dev = DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let elem_state = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("element still present")
        .state;
    assert_eq!(
        elem_state,
        SequenceState::InProgress,
        "launched element must reach InProgress inside the same tick as launch; got {elem_state:?}"
    );
}

#[test]
fn equip_bow_translate_plays_transition_orders() {
    let mut display = HostDisplayState::default();
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let pc_id = engine.add_entity(make_test_pc(Posture::Upright));

    let elem = SequenceElement::new(1, Command::EquipBow, Some(pc_id));
    let seq_id = engine.launch_element(elem);
    engine.ensure_wait_element(pc_id);

    let mut assets = LevelAssets::new();
    let mut dev = DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let elem = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("EquipBow element still present");
    assert_eq!(elem.state, SequenceState::InProgress);
    assert_eq!(
        elem.action_state_after_transition,
        ActionState::AimingWithBow
    );
    assert!(
        elem.orders
            .iter()
            .any(|order| order.order_type == OrderType::TransitionEquipBow),
        "EquipBow should queue the take-bow transition"
    );
    assert!(
        elem.orders
            .iter()
            .any(|order| order.order_type == OrderType::TransitionLoadingBow),
        "EquipBow should queue the loading transition"
    );
}

// ─── NPC translate dispatch ────────────────────────────────────────
//
// The four NPC-specific commands each push a single one-shot
// animation order with `compute_direction = false` and bind sequence
// termination to its DONE.

/// Drive `perform_hourglass` once, asserting the launched element
/// pushed the expected animation onto its order queue and that the
/// order is what the animation driver sees via `current_order_for_actor`.
/// `BEGGAR_SHOW_FACE` runs against a civilian (only civilians can be
/// beggars); the others use a soldier.
fn assert_npc_translate_books(
    command: crate::element::Command,
    expected_anim: crate::order::OrderType,
) {
    let mut dev = DevState::default();
    let mut display = HostDisplayState::default();
    let mut assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let actor = match command {
        crate::element::Command::BeggarShowFace => {
            let actor = engine.add_entity(make_test_civilian(crate::element::Posture::Upright));
            let Entity::Civilian(civilian) = engine
                .get_entity_mut(actor)
                .expect("new BeggarShowFace civilian should exist")
            else {
                panic!("new BeggarShowFace actor should be a civilian");
            };
            civilian.npc.ai_brain = crate::element::AiBrain::Friendly(Box::new(
                crate::ai_friendly::FriendlyAi::new(actor.index()),
            ));
            actor
        }
        _ => engine.add_entity(make_test_soldier(crate::element::Posture::Upright)),
    };

    let elem = crate::sequence::SequenceElement::new(1, command, Some(actor));
    let seq_id = engine.launch_element(elem);
    engine.ensure_wait_element(actor);

    complete_test_runtime_fixture(&mut engine, &mut assets);
    let _ = engine.perform_hourglass(&mut display, &assets, &mut dev);

    let (order_seq, _, order_type) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(actor)
        .map(|(s, e, o)| (s, e, o.order_type))
        .expect("front order should be set");
    assert_eq!(
        order_seq, seq_id,
        "front order should live on the launched element for {command:?}",
    );
    assert_eq!(
        order_type, expected_anim,
        "wrong animation queued for {command:?}",
    );
    let elem_state = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("element present")
        .state;
    assert_eq!(
        elem_state,
        crate::sequence::SequenceState::InProgress,
        "element should stay InProgress while the anim is playing",
    );
}

#[test]
fn wake_up_translate_books_turning_then_waking_up_with_antagonist() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;

    let mut dev = DevState::default();
    let mut display = HostDisplayState::default();
    let mut assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let rescuer = engine.add_entity(make_test_pc(Posture::Upright));
    let target = engine.add_entity(make_test_soldier(Posture::Lying));

    bind_test_action_point(
        &mut engine,
        rescuer,
        OrderType::WakingUp,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );

    let elem = SequenceElement::new_interaction(1, Command::WakeUp, Some(rescuer), Some(target));
    let seq_id = engine.launch_element(elem);
    engine.ensure_wait_element(rescuer);

    complete_test_runtime_fixture(&mut engine, &mut assets);
    let _ = engine.perform_hourglass(&mut display, &assets, &mut dev);

    let (order_seq, order_elem, order) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(rescuer)
        .expect("WakeUp should queue an animation order");
    assert_eq!(order_seq, seq_id);
    assert_eq!(order.order_type, OrderType::Turning);
    let orders = &engine
        .orders
        .sequence_manager
        .get_element(order_seq, order_elem)
        .unwrap()
        .orders;
    assert_eq!(orders.len(), 2);
    assert_eq!(orders[0].order_type, OrderType::Turning);
    assert!(orders[0].compute_direction);
    assert_eq!(orders[1].order_type, OrderType::WakingUp);
    assert_eq!(orders[1].antagonist, Some(target));
}

#[test]
fn waking_up_done_clears_target_concussion_and_waits() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use super::animation::{AnimCompletionOutcomes, ExecuteSideOutcomes};
    use crate::combat::CONCUSSION_THRESHOLD;
    use crate::element::{ActionState, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceState;

    let mut engine = EngineInner::new();
    let rescuer = engine.add_entity(make_test_pc(Posture::Upright));
    let target = engine.add_entity(make_test_soldier(Posture::Lying));
    {
        let target_entity = engine.get_entity_mut(target).expect("target present");
        target_entity.human_data_mut().unwrap().unconscious = true;
        target_entity
            .human_data_mut()
            .unwrap()
            .concussion_of_the_brain = CONCUSSION_THRESHOLD;
        target_entity.npc_data_mut().unwrap().life_points = 30;
        target_entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
    }

    let outcomes = AnimCompletionOutcomes {
        execute_sides: ExecuteSideOutcomes {
            waking_up_done: vec![(rescuer, target)],
            ..Default::default()
        },
        ..Default::default()
    };
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .soldiers
        .resize_with(1, crate::profiles::SoldierProfile::default);

    // The wake target already owns the ordinary unconscious idle Wait.
    // Original target->Wait() must replace this equal-priority element,
    // rather than merely ensuring that some Wait exists.
    let stale_wait = engine.actor_wait(target);
    engine
        .drain_script_synchronous_actions(sim, &assets, &mut Vec::new())
        .expect("initial unconscious Wait should translate synchronously");
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(target)
            .map(|(_, _, order)| order.order_type),
        Some(OrderType::BeingUnconscious)
    );

    engine.process_anim_completion_outcomes(sim, outcomes, &assets);
    engine
        .drain_script_synchronous_actions(sim, &assets, &mut Vec::new())
        .expect("wake completion's fresh Wait should translate synchronously");

    let target_entity = engine.get_entity(target).expect("target present");
    assert_eq!(target_entity.element_data().posture, Posture::Lying);
    assert_eq!(
        target_entity.human_data().unwrap().concussion_of_the_brain,
        0
    );
    assert!(!target_entity.human_data().unwrap().unconscious);
    // The recovery Wait has translated (StandingUp is current below) but its
    // animation has not reached the START edge yet — posture and action
    // state both flip to Upright/Waiting only on that edge, so the actor
    // still reports the pre-wake movement state at this boundary.
    assert_eq!(
        target_entity.actor_data().unwrap().action_state,
        ActionState::Moving
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(stale_wait, 0)
            .expect("stale unconscious Wait remains inspectable")
            .state,
        SequenceState::Interrupted,
        "fresh target->Wait() replaces the stale unconscious idle"
    );
    let (fresh_wait, current_order) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(target)
        .map(|(seq_id, _, order)| (seq_id, order.order_type))
        .expect("fresh recovery Wait should be current");
    assert_eq!(current_order, OrderType::StandingUp);
    assert_ne!(fresh_wait, stale_wait);
}

/// `Point` → `Pointing` animation.
#[test]
fn npc_translate_point_books_pointing_anim() {
    use crate::element::Command;
    use crate::order::OrderType;
    assert_npc_translate_books(Command::Point, OrderType::Pointing);
}

/// `SitDown` → `TransitionWaitingUprightSitting` animation.
#[test]
fn npc_translate_sit_down_books_sit_transition() {
    use crate::element::Command;
    use crate::order::OrderType;
    assert_npc_translate_books(Command::SitDown, OrderType::TransitionWaitingUprightSitting);
}

/// `BeggarShowFace` → `BeggarShowingFace` animation.  Targets a
/// civilian, since only civilians can be beggars.
#[test]
fn npc_translate_beggar_show_face_books_show_face_anim() {
    use crate::element::Command;
    use crate::order::OrderType;
    assert_npc_translate_books(Command::BeggarShowFace, OrderType::BeggarShowingFace);
}

/// `EnterLeisure` → `TransitionWaitingUprightSpecial` animation.
#[test]
fn npc_translate_enter_leisure_books_special_transition() {
    use crate::element::Command;
    use crate::order::OrderType;
    assert_npc_translate_books(
        Command::EnterLeisure,
        OrderType::TransitionWaitingUprightSpecial,
    );
}

#[test]
fn get_killed_at_bottom_kills_lying_victim_immediately() {
    use crate::element::{Command, Posture};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let killer = engine.add_entity(make_test_soldier(Posture::Upright));
    let victim = engine.add_entity(make_test_soldier(Posture::Lying));
    if let Some(crate::element::Entity::Soldier(soldier)) = engine.world.entities.get_mut(victim) {
        soldier.npc.life_points = 30;
        soldier.soldier.cached_max_life_points = 30;
        soldier.human.unconscious = true;
    }

    let elem =
        SequenceElement::new_interaction(1, Command::GetKilledAtBottom, Some(victim), Some(killer));
    engine.launch_element(elem);
    engine.ensure_wait_element(victim);

    let mut display = HostDisplayState::default();
    let mut assets = LevelAssets::new();
    let mut dev = DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let entity = engine.get_entity(victim).expect("victim still present");
    assert!(entity.is_dead());
    assert_eq!(entity.element_data().posture, Posture::DeadBack);
}

/// When the `TransitionWaitingUprightSitting` animation completes,
/// the actor's posture flips to `Sitting`.
#[test]
fn npc_sit_down_anim_completion_flips_posture_to_sitting() {
    use super::animation::{ExecuteSideOutcomes, apply_npc_execute_side_effects};
    use crate::element::{ActionState, EntityId, Posture};
    use crate::order::OrderType;
    use crate::sprite::MotionState;

    let mut entity = make_test_soldier(Posture::Upright);
    let mut outcomes = ExecuteSideOutcomes::default();

    apply_npc_execute_side_effects(
        &mut entity,
        OrderType::TransitionWaitingUprightSitting,
        MotionState::Terminated,
        None,
        EntityId::Pc(crate::entity_id::PcId(0)),
        &mut outcomes,
    );

    assert_eq!(entity.element_data().posture, Posture::Sitting);
    assert_eq!(
        entity.actor_data().expect("actor data").action_state,
        ActionState::Waiting,
    );
}

/// A sitting NPC who receives `Point` first stands up: the auto-leave
/// path snaps the posture to `Upright` and queues the
/// `TransitionSittingWaitingUpright` animation on the actor's
/// `order_queue` so the visible stand-up plays before the gesture.
#[test]
fn sitting_npc_point_auto_stands_up() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_test_soldier(Posture::Sitting));

    let changed = engine.auto_leave_disguise_if_needed(actor, Command::Point);
    assert!(changed, "auto-leave should fire for Sitting + Point");

    let entity = engine.get_entity(actor).expect("entity present");
    assert_eq!(entity.element_data().posture, Posture::Upright);

    let next_order = engine
        .orders
        .sequence_manager
        .current_order_for_actor(actor)
        .map(|(_, _, o)| o.order_type);
    assert_eq!(
        next_order,
        Some(OrderType::TransitionSittingWaitingUpright),
        "stand-up transition should be queued on the owning sequence element",
    );
}

/// `EnterLeisure` on an already-leisuring NPC must not auto-leave
/// leisure first — `GetTransitionFlags` sets
/// `CHANGEPOSTURE_CAN_BE_LEISURING` for this command.
#[test]
fn enter_leisure_on_leisuring_npc_skips_auto_leave() {
    use crate::element::{Command, Posture};

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_test_soldier(Posture::Leisure));

    let changed = engine.auto_leave_disguise_if_needed(actor, Command::EnterLeisure);
    assert!(
        !changed,
        "leisure-leisure re-entry should be a no-op (CAN_BE_LEISURING exempt)",
    );

    let entity = engine.get_entity(actor).expect("entity present");
    assert_eq!(entity.element_data().posture, Posture::Leisure);
    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(actor)
            .is_none(),
        "no transition animation should be queued",
    );
}

/// When the `TransitionWaitingUprightSpecial` animation completes,
/// the actor's posture flips to `Leisure`.
#[test]
fn npc_enter_leisure_anim_completion_flips_posture_to_leisure() {
    use super::animation::{ExecuteSideOutcomes, apply_npc_execute_side_effects};
    use crate::element::{ActionState, EntityId, Posture};
    use crate::order::OrderType;
    use crate::sprite::MotionState;

    let mut entity = make_test_soldier(Posture::Upright);
    let mut outcomes = ExecuteSideOutcomes::default();

    apply_npc_execute_side_effects(
        &mut entity,
        OrderType::TransitionWaitingUprightSpecial,
        MotionState::Done,
        None,
        EntityId::Pc(crate::entity_id::PcId(0)),
        &mut outcomes,
    );

    assert_eq!(entity.element_data().posture, Posture::Leisure);
    assert_eq!(
        entity.actor_data().expect("actor data").action_state,
        ActionState::Waiting,
    );
}

/// `remove_quick_action_titbits_for(pc, level)` looks up the
/// per-level titbit entry on the PC, drops every titbit with that id,
/// and reports whether anything was removed.
#[test]
fn remove_quick_action_titbits_for_matches_original_signature() {
    use crate::coordinates::WorldPoint3D;
    use crate::element::EntityId;
    use crate::titbit::{ElementHandle, INVALID_ID, QuickAction, TitbitKind};

    let mut engine = EngineInner::new();
    let pc = EntityId::Pc(crate::entity_id::PcId(42));
    let slot: u8 = 1;

    // Empty slot → early-returns on the sentinel id.
    assert!(!engine.remove_quick_action_titbits_for(pc, slot));

    // Add a QA titbit and wire its id into the PC's macro slot.
    let pc_handle = ElementHandle(pc.index());
    let titbit_id = engine.feedback.titbit_manager.add_titbit(
        WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        0,
        TitbitKind::QuickAction,
        pc_handle,
        QuickAction::Bow as u16,
        pc_handle,
        false,
        INVALID_ID,
        true,
        Some(0.0),
        Some(0),
    );
    assert_ne!(titbit_id, INVALID_ID);
    engine
        .players
        .macro_store
        .get_or_insert(pc)
        .set_slot_titbit(
            slot as usize,
            crate::titbit::TitbitId::new(titbit_id).unwrap(),
        );

    // Populated slot → drops the titbit and reports success.
    assert!(engine.remove_quick_action_titbits_for(pc, slot));
    assert!(
        !engine
            .feedback
            .titbit_manager
            .titbits()
            .iter()
            .any(|t| t.id == titbit_id),
        "titbit with id {titbit_id} should be gone"
    );

    // Second call after the list is empty: slot still holds the stale
    // id (the caller clears the level entry after this returns), but
    // no titbit matches, so it returns false.
    assert!(!engine.remove_quick_action_titbits_for(pc, slot));
}
