use super::*;

#[test]
fn zoom_dispatch_uses_supplied_camera_display_not_owned_placeholder() {
    for request in [
        EngineStateRequest::ZoomingUp,
        EngineStateRequest::ZoomingDown,
    ] {
        for supplied_is_idle in [false, true] {
            let mut engine = EngineInner::new();
            engine.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);
            engine.feedback.cutscene_camera.zoom_factor = 1.0;
            engine.feedback.cutscene_camera.zoom_init_done = false;
            let owned_op = if supplied_is_idle {
                DisplayOpCode::InZoom
            } else {
                DisplayOpCode::NoBackgroundMove
            };
            engine.feedback.cutscene_camera.display.display_op = owned_op;
            let mut supplied = CameraDisplayState::default();
            supplied.display_op = if supplied_is_idle {
                DisplayOpCode::NoBackgroundMove
            } else {
                DisplayOpCode::InZoom
            };
            supplied.background_transform.current_zoom_level = 1;

            assert_eq!(engine.is_zoom_possible(), !supplied_is_idle);
            assert_eq!(
                engine.is_zoom_possible_for_camera(&supplied),
                supplied_is_idle
            );
            assert_eq!(
                engine.change_state(&mut supplied, 0, request),
                supplied_is_idle
            );
            assert_eq!(
                supplied.display_op,
                if supplied_is_idle {
                    DisplayOpCode::InitZoom
                } else {
                    DisplayOpCode::InZoom
                }
            );
            assert_eq!(engine.feedback.cutscene_camera.display.display_op, owned_op);
        }
    }
}

#[test]
fn hourglass_returns_in_progress() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let result = engine
        .perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev)
        .code;
    assert_eq!(result, GameCode::LevelInProgress);
    assert_eq!(engine.control.frame_counter, 1);
}

#[test]
fn hourglass_phase_trace_records_only_phases_reached_before_mission_exit() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.mission_domain.state.quit_won = true;

    begin_hourglass_phase_capture();
    let result = engine
        .perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev)
        .code;
    let phases = end_hourglass_phase_capture();

    assert_eq!(result, GameCode::LevelSucceeded);
    assert_eq!(
        phases,
        vec![
            HourglassPhase::DeferredEffectsStart,
            HourglassPhase::MissionAndMessages,
        ]
    );
}

#[test]
fn blocking_fade_frame_runs_before_rng_clock_and_phase_dispatch() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.set_fade_freeze_frames_remaining(1);
    let rng_seed = engine.rng_seed();

    begin_hourglass_phase_capture();
    let result = engine
        .perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev)
        .code;
    let phases = end_hourglass_phase_capture();

    assert_eq!(result, GameCode::LevelInProgress);
    assert_eq!(engine.control.frame_counter, 0);
    assert_eq!(engine.rng_seed(), rng_seed);
    assert!(phases.is_empty());
    assert_eq!(engine.fade_freeze_frames_remaining(), 0);
}

#[test]
fn pending_sequence_animation_starts_after_entity_hourglass_boundary() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Upright));
    bind_test_action_point(
        &mut engine,
        soldier_id,
        OrderType::TransitionWaitingUprightSitting,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );
    // Bypass EngineInner's synchronous launch wrapper to model an element
    // already waiting in the sequence manager's FIFO at frame start.
    let mut element = SequenceElement::new(1, Command::SitDown, Some(soldier_id));
    element.posture_after_transition = Posture::Upright;
    let sequence_id = engine.orders.sequence_manager.launch_element(element);

    let mut display = HostDisplayState::default();
    let mut assets = LevelAssets::new();
    let mut dev = DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);

    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence_id, 0)
        .expect("pending element should have dispatched");
    assert_eq!(element.state, SequenceState::InProgress);
    let order_id = element
        .current_order()
        .expect("SitDown should translate to an animation")
        .order_id
        .get();
    assert_eq!(
        engine
            .get_entity(soldier_id)
            .expect("soldier present")
            .element_data()
            .sprite
            .last_processed_order_id,
        u32::MAX,
        "an order dispatched by the sequence manager after the entity loop must not animate in that same frame"
    );

    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
    assert_eq!(
        engine
            .get_entity(soldier_id)
            .expect("soldier present")
            .element_data()
            .sprite
            .last_processed_order_id,
        order_id,
        "the dispatched animation must start on the following entity frame"
    );
}

#[test]
#[should_panic(expected = "Entity::Net has invalid ObjectType::None")]
fn inactive_unsupported_net_mapping_panics_before_owner_slot_retention() {
    let mut engine = EngineInner::new();
    engine.add_entity(Entity::Net(crate::element::ElementNet {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::ObjectNet;
            initial_element.active = false;
            initial_element
        },
        object: crate::element::ObjectData {
            object_type: crate::element::ObjectType::None,
            ..Default::default()
        },
        projectile: Default::default(),
        net: Default::default(),
    }));
    engine.perform_hourglass(
        &mut HostDisplayState::default(),
        &mut InputState::default(),
        &LevelAssets::new(),
        &mut DevState::default(),
    );
}

#[test]
fn carried_corpse_transition_drops_before_following_whistle_order() {
    use crate::element::{Command, Posture};
    use crate::movement::AbilityKind;
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    // The body owns an earlier legacy creation slot, as in the replay. Its
    // outdoor delayed landing therefore cannot commit until the next frame.
    let carried = engine.add_entity(make_test_soldier(Posture::Carried));
    let carrier = engine.add_entity(make_test_pc(Posture::CarryingCorpse));
    {
        let entity = engine.get_entity_mut(carrier).unwrap();
        let pc = entity.pc_data_mut().unwrap();
        pc.carried = Some(carried);
        pc.set_live_carried_posture(Posture::Lying);
    }
    {
        let entity = engine.get_entity_mut(carried).unwrap();
        entity.human_data_mut().unwrap().carrier = Some(carrier);
        entity.actor_data_mut().unwrap().execution_frozen = true;
    }

    let dropped = OrderType::BeingDroppedPeasantC;
    let dropped_script = SpriteScript {
        action_id: dropped as u16,
        action_done: 0,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1],
        delays: vec![0],
        distances: vec![0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
        sound_ids: vec![0],
    };
    let mut dropped_conversion = vec![UNMAPPED; NONANIMATION_END];
    dropped_conversion[dropped as usize] = 0;
    let dropped_sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![dropped_script; 16]),
        std::sync::Arc::new(dropped_conversion),
    );
    engine
        .get_entity_mut(carried)
        .unwrap()
        .element_data_mut()
        .sprite = dropped_sprite;

    let transition = OrderType::TransitionCarryingCorpseWaitingUpright;
    let script = SpriteScript {
        action_id: transition as u16,
        action_done: 0,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1],
        delays: vec![0],
        distances: vec![0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
        sound_ids: vec![0],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[transition as usize] = 0;
    let sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    engine
        .get_entity_mut(carrier)
        .unwrap()
        .element_data_mut()
        .sprite = sprite;
    engine
        .get_entity_mut(carrier)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(13);

    let transition_id = engine.orders.allocate_order_id();
    let transition_order = Order::new(transition, 0.0, 0.0, transition_id);
    let whistle_id = engine.orders.allocate_order_id();
    let whistle_order = Order::new(OrderType::Whistling, 0.0, 0.0, whistle_id);
    let mut element = SequenceElement::new(1, Command::WhistleCmd, Some(carrier));
    element.orders.push_back(transition_order);
    element.orders.push_back(whistle_order);
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    {
        let actor = engine
            .get_entity_mut(carrier)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.active_ability.kind = Some(AbilityKind::Whistle);
        actor.active_ability.sequence_id = Some(sequence);
        actor.active_ability.element_index = 0;
        actor.active_ability.order_id = Some(whistle_id);
    }

    let sim = crate::sim_rng::test_context();
    let assets = assets_with_test_pc_profile();
    for _ in 0..8 {
        engine.tick_actor_animation_action_change_slots(&sim, &assets);
        if engine.get_entity(carrier).unwrap().posture() == Posture::Upright {
            break;
        }
    }

    let carrier_entity = engine.get_entity(carrier).unwrap();
    assert_eq!(carrier_entity.posture(), Posture::Upright);
    assert_eq!(carrier_entity.pc_data().unwrap().carried, None);
    let carried_entity = engine.get_entity(carried).unwrap();
    assert_eq!(carried_entity.posture(), Posture::Lying);
    assert_eq!(carried_entity.human_data().unwrap().carrier, None);
    assert!(!carried_entity.actor_data().unwrap().execution_frozen);
    assert!(
        carried_entity.element_data().position_map_delayed,
        "outdoor DropCorpse must queue its landing after the body's earlier creation slot"
    );
    assert_eq!(carried_entity.sprite().last_action, dropped);
    assert_eq!(
        carried_entity.sprite().frame_count,
        carrier_entity.sprite().frame_count,
        "terminal animation synchronization must run before dropping the body unlinks the pair"
    );
    assert_eq!(carried_entity.element_data().direction(), 9);
    assert_eq!(
        i16::from(carried_entity.position_iface().get_direction_goal()),
        13,
        "dropping a corpse must preserve the distinct carrier-facing goal after clearing its carrier"
    );
    let (_, _, selected) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(carrier)
        .expect("following Whistle order must remain selected");
    assert_eq!(selected.order_type, OrderType::Whistling);
    assert_ne!(selected.order_id, transition_id);
}

#[test]
fn selected_action_stop_drops_mid_grab_before_the_body_actor_slot() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::player_command::PlayerCommand;
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let body = engine.add_entity(make_test_soldier(Posture::Tied));
    let carrier = engine.add_entity(make_test_pc(Posture::Upright));
    {
        let carrier_entity = engine.get_entity_mut(carrier).unwrap();
        carrier_entity.pc_data_mut().unwrap().carried = Some(body);
        carrier_entity
            .pc_data_mut()
            .unwrap()
            .set_live_carried_posture(Posture::Tied);
    }
    {
        let tied = OrderType::BeingTied;
        let script = SpriteScript {
            action_id: tied as u16,
            action_done: 0,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[tied as usize] = 0;
        let body_entity = engine.get_entity_mut(body).unwrap();
        body_entity.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        body_entity.human_data_mut().unwrap().carrier = Some(carrier);
        body_entity.human_data_mut().unwrap().unconscious = true;
        body_entity.actor_data_mut().unwrap().execution_frozen = true;
    }

    let order_id = engine.orders.allocate_order_id();
    let mut take =
        SequenceElement::new_interaction(1, Command::TakeCorpse, Some(carrier), Some(body));
    take.orders.push_back(Order::new(
        OrderType::TransitionWaitingUprightCarryingCorpse,
        0.0,
        0.0,
        order_id,
    ));
    let take_sequence = engine.orders.sequence_manager.launch_element(take);
    engine
        .orders
        .sequence_manager
        .element_in_progress(take_sequence, 0);
    engine.players.seats[0].selection = vec![carrier];

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut display = crate::engine::HostDisplayState::default();
    let mut input = InputState::default();
    engine.apply_command(
        &sim,
        &mut display,
        &mut input,
        &assets,
        &PlayerCommand::SelectResolvedAction {
            pc_id: carrier,
            action: crate::profiles::Action::HelpToClimb,
        },
    );

    assert_eq!(
        engine
            .get_entity(body)
            .unwrap()
            .human_data()
            .unwrap()
            .carrier,
        None,
        "SelectAction's synchronous stop completion must release the body before the update"
    );
    let selected = engine
        .orders
        .sequence_manager
        .current_element_for_actor(body)
        .and_then(|(sequence, index)| engine.orders.sequence_manager.get_element(sequence, index))
        .expect("the released body must already own its Wait");
    assert_eq!(selected.command, Command::Wait);
    let expected_order_id = selected.current_order().map(|order| order.order_id);

    engine.tick_actor_animation_action_change_slots(&sim, &assets);
    let body_entity = engine.get_entity(body).unwrap();
    assert_eq!(body_entity.sprite().last_action, OrderType::BeingTied);
    assert_eq!(
        body_entity.actor_data().unwrap().last_execute_order_id,
        expected_order_id,
        "the body's creation-order slot must execute the freshly installed tied idle"
    );
}

#[test]
fn inactive_actor_hourglass_installs_and_advances_idle_wait() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite::MotionState;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .active = false;

    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .is_none(),
        "regression requires the actor update to synthesize the idle Wait"
    );

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    engine.tick_actor_animation_action_change_slots(&sim, &assets);

    let entity = engine.get_entity(owner).unwrap();
    assert!(!entity.is_active());
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
            .and_then(|(seq, index)| engine.orders.sequence_manager.get_element(seq, index))
            .map(|element| element.command),
        Some(Command::Wait),
        "inactive actor updates must lazily install the same Wait as an active actor"
    );

    let animated = engine.add_entity(make_test_pc(Posture::Upright));
    let script = SpriteScript {
        action_id: OrderType::WaitingUpright as u16,
        action_done: 3,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3, 4],
        delays: vec![2; 4],
        distances: vec![0; 4],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 4],
        sound_ids: vec![0; 4],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::WaitingUpright as usize] = 0;
    let entity = engine.get_entity_mut(animated).unwrap();
    entity.element_data_mut().active = false;
    entity.actor_data_mut().unwrap().action_state = crate::element::ActionState::Waiting;
    entity.element_data_mut().sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(conversion),
    );

    let mut selected = SequenceElement::new(1, Command::Wait, Some(animated));
    selected
        .orders
        .push_back(Order::test_new(OrderType::WaitingUpright, 0.0, 0.0));
    let sequence = engine.orders.sequence_manager.launch_element(selected);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    let mut executed_idle = false;
    let mut forwarded_termination = false;
    for _ in 0..16 {
        let (_, _, executed) = engine.tick_actor_animation_for(&sim, &assets, animated);
        executed_idle |= executed.is_some();
        forwarded_termination |= executed
            .as_ref()
            .is_some_and(|result| result.motion == MotionState::Terminated);
        if forwarded_termination {
            break;
        }
    }
    let entity = engine.get_entity(animated).unwrap();
    assert!(
        executed_idle,
        "inactive actor updates must execute the selected idle order"
    );
    assert_eq!(entity.sprite().last_action, OrderType::WaitingUpright);
    assert!(
        forwarded_termination,
        "WaitingUpright must forward sprite termination so the update can advance into the bored transition"
    );
}

#[test]
fn unconscious_tied_wait_keeps_advancing_its_hold_animation() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite::MotionState;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Tied));
    let mut selected = SequenceElement::new(1, Command::Wait, Some(owner));
    let order = Order::test_new(OrderType::BeingTied, 0.0, 0.0);
    let order_id = order.order_id;
    selected.orders.push_back(order);
    let sequence = engine.orders.sequence_manager.launch_element(selected);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    let script = SpriteScript {
        action_id: OrderType::BeingTied as u16,
        action_done: 0,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1],
        delays: vec![1],
        distances: vec![0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
        sound_ids: vec![0],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::BeingTied as usize] = 0;
    let entity = engine.get_entity_mut(owner).unwrap();
    entity.human_data_mut().unwrap().unconscious = true;
    entity.actor_data_mut().unwrap().action_state = ActionState::Waiting;
    entity.element_data_mut().sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(conversion),
    );
    entity.element_data_mut().sprite.last_action = OrderType::BeingTied;
    entity.element_data_mut().sprite.last_processed_order_id = order_id.get();

    let (_, _, executed) = engine.tick_actor_animation_for(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
    );

    assert_eq!(
        executed.map(|result| result.motion),
        Some(MotionState::InProgress),
        "BeingTied holds human execution even though tied humans carry the unconscious flag"
    );
    // Each tick slot runs exactly one action-processing step: the fresh
    // sprite's 0xFFFF frame-count sentinel wraps to 0 on the first tick and
    // advances to 1 on the second. A frozen hold would leave it untouched.
    assert_eq!(
        engine.get_entity(owner).unwrap().sprite().frame_count,
        0,
        "the tied hold must advance by one action step on its first tick"
    );
    engine.tick_actor_animation_for(&crate::sim_rng::test_context(), &LevelAssets::new(), owner);
    assert_eq!(
        engine.get_entity(owner).unwrap().sprite().frame_count,
        1,
        "the tied hold must retain the original game's per-tick action processing"
    );
}

#[test]
fn face_to_waits_for_manager_regardless_of_owner_drain_mode() {
    use crate::coordinates::MapPoint;
    use crate::element::{ActionState, Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{AiOrderIntent, Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};
    use std::num::NonZeroU32;

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    let mut soldier = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier_data) = &mut soldier else {
        unreachable!("make_test_soldier returned a non-soldier")
    };
    soldier_data.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    let owner = engine.add_entity(soldier);
    let mut movement =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    movement.priority = SequencePriority::Normal;
    movement.orders.push_back(Order::new(
        OrderType::WalkingUpright,
        70.0,
        80.0,
        NonZeroU32::new(777).unwrap(),
    ));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        let actor = entity.actor_data_mut().unwrap();
        actor.action_state = ActionState::Moving;
        actor.active_movement = ActiveMovement::new(movement_sequence, 0);
        entity
            .position_iface_mut()
            .set_map_goal(MapPoint::new(70.0, 80.0));
    }
    engine
        .get_entity_mut(owner)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .outbox
        .actor
        .halt = true;
    engine
        .get_entity_mut(owner)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .outbox
        .actor
        .orders
        .push(AiOrderIntent::face_direction(9));

    engine.launch_pending_orders_for_npc(&sim, &assets, owner);

    let turn_sequence = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .find(|sequence| {
            sequence.elements.first().is_some_and(|element| {
                element.owner == Some(owner) && element.command == Command::Turn
            })
        })
        .expect("deferred standalone facing registered a turn");
    assert_eq!(turn_sequence.elements[0].state, SequenceState::Todo);
    assert!(
        turn_sequence.elements[0].orders.is_empty(),
        "an ordinary walking actor must not execute or translate an AI-tail Turn in the same owner slot"
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        MapPoint::ZERO,
        "an explicit StopAll before Face must not resurrect the stopped movement goal"
    );
    let turn_sequence_id = turn_sequence.id;

    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let instructed = engine
        .orders
        .sequence_manager
        .get_element(turn_sequence_id, 0)
        .unwrap();
    assert_eq!(instructed.state, SequenceState::InProgress);
    assert_eq!(
        instructed
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![
            OrderType::TransitionWalkingUprightWaitingUpright,
            OrderType::Turning,
        ]
    );
}

#[test]
fn ordered_ability_dispatch_does_not_advance_a_later_actor() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let first = engine.add_entity(make_test_pc(Posture::Upright));
    let second = engine.add_entity(make_test_pc(Posture::Upright));
    for actor_id in [first, second] {
        bind_test_action_point(
            &mut engine,
            actor_id,
            OrderType::Eating,
            crate::coordinates::SpriteLocalPoint::ZERO,
            crate::coordinates::SpriteAnchor::ZERO,
        );
        let sequence_id = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::EatCmd, Some(actor_id)));
        assert_eq!(
            crate::abilities::begin_eat(
                &mut engine.world.entities,
                &mut engine.orders.sequence_manager,
                actor_id,
                sequence_id,
                0,
                &mut engine.orders.next_order_id,
            ),
            crate::abilities::BeginResult::Started
        );
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);
    }

    let mut display = CameraDisplayState::default();
    let assets = LevelAssets::new();
    engine.tick_ability_for(sim, &mut display, &assets, first);

    assert_ne!(
        engine
            .get_entity(first)
            .expect("first ability actor present")
            .element_data()
            .sprite
            .last_processed_order_id,
        u32::MAX,
        "the actor at the current creation slot must advance"
    );
    assert_eq!(
        engine
            .get_entity(second)
            .expect("later ability actor present")
            .element_data()
            .sprite
            .last_processed_order_id,
        u32::MAX,
        "a later actor's ability cannot advance from an earlier actor's update"
    );
}

#[test]
fn invalid_eat_initialization_short_circuits_the_full_execute_owner_slot() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};

    let mut description = crate::campaign::PcDescription {
        character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
        ..Default::default()
    };
    description.status.num_rations = 1;
    let mut campaign = crate::campaign::Campaign::default();
    campaign.characters.push(description);

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new_with_campaign(campaign);
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    {
        let pc = engine.get_entity_mut(owner).unwrap().pc_data_mut().unwrap();
        pc.campaign_description_index = Some(0);
        pc.life_points = crate::pc_status::LIFEPOINTS_PC;
    }
    bind_test_action_point(
        &mut engine,
        owner,
        OrderType::Eating,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );

    let sequence = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::EatCmd, Some(owner)));
    assert_eq!(
        crate::abilities::begin_eat(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            owner,
            sequence,
            0,
            &mut engine.orders.next_order_id,
        ),
        crate::abilities::BeginResult::Started
    );
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let sprite_before = {
        let sprite = engine.get_entity(owner).unwrap().sprite();
        (
            sprite.current_row,
            sprite.current_frame,
            sprite.frame_count,
            sprite.last_processed_order_id,
        )
    };
    let mut selected_ability = None;
    engine.tick_actor_animation_action_change_slots_with_hooks(
        &sim,
        &assets,
        |_, _| {},
        |_, _| {},
        |_, selected_owner, _, _, _, ability, _| {
            if selected_owner == owner {
                selected_ability = Some(ability);
            }
        },
        |_, _, _| {},
    );

    assert_eq!(selected_ability, Some(None));
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
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        crate::sprite::MotionState::Terminated
    );
    let sprite = engine.get_entity(owner).unwrap().sprite();
    assert_eq!(
        (
            sprite.current_row,
            sprite.current_frame,
            sprite.frame_count,
            sprite.last_processed_order_id,
        ),
        sprite_before,
        "Eating's invalid initialization returns before its sprite body"
    );
    assert_eq!(engine.campaign().characters[0].status.num_rations, 1);
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .pc_data()
            .unwrap()
            .life_points,
        crate::pc_status::LIFEPOINTS_PC,
        "the rejected ability must apply neither ammo nor healing side effects"
    );
}

#[test]
fn instant_shield_raise_remains_selected_until_redundant_current_owner_raise_replaces_it() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};

    let sim = crate::sim_rng::test_context();
    let mut assets = LevelAssets::default();
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    profiles.soldiers.push(crate::profiles::SoldierProfile {
        hth_weapon_id: 1,
        ..Default::default()
    });
    profiles.hth_weapons.push(Default::default());
    assert!(assets.profile_manager.get_hth_weapon(1).is_some());
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::Waiting;

    let first = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new_generic(
            1,
            Command::RaiseShieldInstantly,
            Some(owner),
        ));
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let first_element = engine
        .orders
        .sequence_manager
        .get_element(first, 0)
        .expect("instant shield raise remains registered");
    assert_eq!(first_element.state, SequenceState::InProgress);
    assert_eq!(
        first_element.current_order().map(|order| order.order_type),
        Some(OrderType::WaitingShield)
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner),
        Some((first, 0)),
        "actor instruction keeps the accepted instant raise selected"
    );

    // A second normal-priority instant raise is a real current-owner control:
    // from HOLDING_SHIELD Original generates LOWERING_SHIELD, interrupts the
    // first normal element, and installs the replacement's WAITING_SHIELD.
    let redundant = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new_generic(
            1,
            Command::RaiseShieldInstantly,
            Some(owner),
        ));
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(redundant, 0)
            .expect("redundant instant raise remains inspectable")
            .state,
        SequenceState::InProgress
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner),
        Some((redundant, 0))
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .map(|(_, _, order)| order.order_type),
        Some(OrderType::LoweringShield)
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(first, 0)
            .expect("replaced instant raise remains inspectable")
            .state,
        SequenceState::Interrupted
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(redundant, 0)
            .unwrap()
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        [OrderType::LoweringShield, OrderType::WaitingShield]
    );
}

#[test]
fn production_receive_purse_reveals_before_advancing_waiting_order_identity() {
    use crate::element::{Command, Posture, ReceivePursePhase};
    use crate::movement::{AbilityKind, ActiveAbility};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let beggar = engine.add_entity(make_test_civilian(Posture::Upright));
    let Entity::Civilian(civilian) = engine.get_entity_mut(beggar).unwrap() else {
        unreachable!()
    };
    civilian.civilian.beggar_scroll_sets = Some(vec![vec![]]);
    let script = SpriteScript {
        action_id: OrderType::WaitingWithPurse as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2],
        delays: vec![0, 0],
        distances: vec![0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
        sound_ids: vec![0; 2],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::WaitingWithPurse as usize] = 0;
    engine
        .get_entity_mut(beggar)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(conversion),
    );
    let mut element = SequenceElement::new(1, Command::ReceivePurse, Some(beggar));
    let waiting = Order::test_new(OrderType::WaitingWithPurse, 0.0, 0.0);
    let waiting_id = waiting.order_id;
    element.orders.push_back(waiting);
    element.orders.push_back(Order::test_new(
        OrderType::TransitionWaitingWithPurseWaitingUpright,
        0.0,
        0.0,
    ));
    let seq = engine.orders.sequence_manager.launch_element(element);
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    let actor = engine
        .get_entity_mut(beggar)
        .unwrap()
        .actor_data_mut()
        .unwrap();
    actor.receive_purse_phase = ReceivePursePhase::Waiting;
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::ReceivePurse),
        sequence_id: Some(seq),
        element_index: 0,
        target: None,
        order_id: Some(waiting_id),
        done_effect_applied: false,
        strangle_initialized: false,
    };

    let observed = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed_hook = observed.clone();
    crate::engine::combat::set_receive_purse_reveal_observer(Some(Box::new(
        move |engine, owner| {
            let (_, _, order) = engine
                .orders
                .sequence_manager
                .current_order_for_actor(owner)
                .expect("ReceivePurse reveal retains its current order");
            observed_hook.set(
                order.order_id == waiting_id && order.order_type == OrderType::WaitingWithPurse,
            );
        },
    )));
    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .civilians
        .push(Default::default());
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for _ in 0..10 {
        engine.tick_actor_owner_envelopes_with_test_owner_hook(
            &crate::sim_rng::test_context(),
            &assets,
            &positions,
            |_, _| {},
        );
        if observed.get() {
            break;
        }
    }
    crate::engine::combat::set_receive_purse_reveal_observer(None);
    assert!(observed.get());
    assert_eq!(
        engine
            .get_entity(beggar)
            .unwrap()
            .actor_data()
            .unwrap()
            .receive_purse_phase,
        ReceivePursePhase::Transition
    );
}

#[test]
fn selected_beggar_entry_stop_leaves_transient_nonanimation_before_next_idle() {
    use crate::element::{ActionState, Posture};
    use crate::order::OrderType;
    use crate::profiles::Action;
    use crate::sequence::{SequenceId, SequenceState};
    use crate::sprite::MotionState;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let pc = engine.add_entity(make_test_pc(Posture::SimulatingBeggar));
    engine.players.seats[0].selection.push(pc);
    engine.players.seats[0].selected_action = Action::Beggar;
    {
        let entity = engine.get_entity_mut(pc).unwrap();
        entity.pc_data_mut().unwrap().current_action = Action::Beggar;
        let actor = entity.actor_data_mut().unwrap();
        actor.action_state = ActionState::Waiting;
        actor.installed_order = None;
        actor.continuation.motion_state = MotionState::Terminated;
    }
    let mut assets = assets_with_test_pc_profile();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    // EnterBeggar DONE has already retired its transition before deferred
    // Execute side effects are drained in Rust. Original still has that
    // transition selected here: Wait is postponed behind it and the following
    // selected-PC SelectAction(Beggar) Stop discards the Wait.
    engine.drain_beggar_wait_handoffs(&sim, &assets, vec![(pc, true)]);

    assert_eq!(
        engine.actor_order_type(pc),
        Some(OrderType::NonanimationEnd)
    );
    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::Terminated
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(pc)
            .is_none(),
        "the callback Wait must not replace the finished entry transition in the same frame"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(SequenceId(1), 0)
            .expect("the stopped callback Wait retains its allocated identity")
            .state,
        SequenceState::Interrupted
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(pc, |command| command
                == crate::element::Command::Wait),
        "the interrupted callback Wait must not survive as live actor work"
    );

    // The actor update sees the null order on the following frame and creates
    // the regular posture-derived Wait, which translates to beggar idle.
    engine.ensure_wait_element(pc);
    engine
        .drain_script_registration_inline_actions(&sim, &assets, &mut Vec::new())
        .unwrap();
    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(pc)
            .is_some_and(|(sequence, _, order)| {
                sequence == SequenceId(2) && order.order_type == OrderType::SimulatingBeggar
            }),
        "the following actor frame must install the normal beggar idle"
    );
}

#[test]
fn selected_beggar_exit_preserves_action_that_replaced_beggar() {
    use crate::element::Posture;
    use crate::profiles::Action;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let pc = engine.add_entity(make_test_pc(Posture::Upright));
    engine.players.seats[0].selection.push(pc);
    engine.players.seats[0].selected_action = Action::Net;
    engine
        .get_entity_mut(pc)
        .unwrap()
        .pc_data_mut()
        .unwrap()
        .current_action = Action::Net;
    let mut assets = assets_with_test_pc_profile();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine.drain_beggar_wait_handoffs(&sim, &assets, vec![(pc, false)]);

    assert_eq!(engine.players.seats[0].selected_action, Action::Net);
    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .pc_data()
            .unwrap()
            .current_action,
        Action::Net,
        "the engine must never see a beggar-action deselection after the messenger rejects its stale action"
    );
}

#[test]
fn selected_beggar_exit_clears_action_while_beggar_is_still_selected() {
    use crate::element::Posture;
    use crate::profiles::Action;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let pc = engine.add_entity(make_test_pc(Posture::Upright));
    engine.players.seats[0].selection.push(pc);
    engine.players.seats[0].selected_action = Action::Beggar;
    engine
        .get_entity_mut(pc)
        .unwrap()
        .pc_data_mut()
        .unwrap()
        .current_action = Action::Beggar;
    let mut assets = assets_with_test_pc_profile();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine.drain_beggar_wait_handoffs(&sim, &assets, vec![(pc, false)]);

    assert_eq!(engine.players.seats[0].selected_action, Action::NoAction);
    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .pc_data()
            .unwrap()
            .current_action,
        Action::NoAction
    );
}

#[test]
fn non_stranglable_terminal_retaliation_falls_through_to_cleanup_and_victim_starts_same_done_tick()
{
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    fn bind(engine: &mut EngineInner, id: EntityId, action: OrderType) {
        let script = SpriteScript {
            action_id: action as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2, 3],
            delays: vec![0, 0, 0],
            distances: vec![0, 0, 0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0; 3],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[action as usize] = 0;
        engine.get_entity_mut(id).unwrap().element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
    }

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let _null_handle_slot = engine.add_entity(make_test_pc(Posture::Upright));
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .active = true;
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .active = true;
    bind(&mut engine, attacker, OrderType::Strangling);
    bind(&mut engine, victim, OrderType::BeingStrangled);
    for id in [attacker, victim] {
        engine
            .get_entity_mut(id)
            .unwrap()
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                crate::coordinates::MapVec::new(-5.0, -5.0),
                crate::coordinates::MapVec::new(5.0, 5.0),
            ));
    }
    let mut assets = LevelAssets::new();
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.characters.push(Default::default());
    profiles.soldiers.push(crate::profiles::SoldierProfile {
        strangle: false,
        ..Default::default()
    });
    assets.profile_manager = std::sync::Arc::new(profiles);
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .npc_data_mut()
        .unwrap()
        .eye_status = crate::element::EyeStatus::LookToTheLeft;
    let seq = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new_interaction(
            1,
            Command::StrangleCmd,
            Some(attacker),
            Some(victim),
        ));
    assert_eq!(
        crate::abilities::begin_strangle(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            attacker,
            victim,
            seq,
            0,
            &mut engine.orders.next_order_id
        ),
        crate::abilities::BeginResult::Started
    );
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_ability
        .strangle_initialized = true;
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    let mut display = CameraDisplayState::default();

    for _ in 0..10 {
        engine.tick_ability_for(&sim, &mut display, &assets, attacker);
        if engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .done_effect_applied
        {
            break;
        }
    }
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .done_effect_applied
    );
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .element_data()
            .sprite
            .last_action,
        OrderType::BeingStrangled,
        "attacker Done must force the victim animation before its same-invocation increment"
    );
    assert!(
        engine
            .get_entity(victim)
            .unwrap()
            .element_data()
            .sprite
            .current_frame
            > 0,
        "victim virgin increment must occur during initial attacker Done setup"
    );
    // Once DONE has latched, a fast-turn short-circuit still executes the
    // original game's strangling tail exactly once. `tick_ability` performs that
    // increment itself; the engine wrapper must not add a second one.
    {
        let attacker_entity = engine.get_entity_mut(attacker).unwrap();
        attacker_entity
            .element_data_mut()
            .set_direction_instantly(0);
        attacker_entity.element_data_mut().set_direction_goal(1);
        let victim_sprite = &mut engine
            .get_entity_mut(victim)
            .unwrap()
            .element_data_mut()
            .sprite;
        victim_sprite.current_frame = 0;
        victim_sprite.frame_count = 0;
    }
    engine.tick_ability_for(&sim, &mut display, &assets, attacker);
    let victim_sprite = &engine.get_entity(victim).unwrap().element_data().sprite;
    assert_eq!(
        (victim_sprite.current_frame, victim_sprite.frame_count),
        (1, 0),
        "the pre-action fast-turn path and normal strangling tail must not both advance the victim"
    );
    engine.dispatch_ai_stimulus(
        victim,
        crate::ai::Stimulus::new(crate::ai::StimulusType::EventTimer),
    );
    engine.dispatch_ai_stimulus(
        victim,
        crate::ai::Stimulus::new(crate::ai::StimulusType::EventFitAgain),
    );

    let (_, condolation_order) =
        crate::engine::soldier_helpers::capture_strangle_condolation_order(|| {
            for _ in 0..10 {
                engine.tick_ability_for(&sim, &mut display, &assets, attacker);
                if !engine
                    .get_entity(attacker)
                    .unwrap()
                    .actor_data()
                    .unwrap()
                    .active_ability
                    .is_active()
                {
                    break;
                }
            }
        });
    assert_eq!(
        condolation_order,
        [
            "TerminalEventGotHit",
            "Wait",
            "Unlock",
            "EventGotHit",
            "LookForward",
        ],
        "both original EventGotHit handler boundaries must complete synchronously in order"
    );
    assert!(
        !engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active(),
        "non-stranglable retaliation must not return before the following Terminated cleanup"
    );
    assert_ne!(
        engine
            .orders
            .sequence_manager
            .get_element(seq, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::InProgress
    );
    let victim_waits: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| {
            sequence
                .elements
                .iter()
                .map(move |element| (sequence.id, element))
        })
        .filter(|(_, element)| element.owner == Some(victim) && element.command == Command::Wait)
        .collect();
    assert_eq!(victim_waits.len(), 1);
    assert!(victim_waits[0].0 > seq);
    let victim_entity = engine.get_entity(victim).unwrap();
    assert!(!victim_entity.ai_controller().unwrap().ai_is_locked());
    assert_eq!(
        victim_entity.npc_data().unwrap().eye_status,
        crate::element::EyeStatus::LookForward
    );
    assert_eq!(
        victim_entity
            .ai_controller()
            .unwrap()
            .outbox
            .detection
            .stimuli
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![
            crate::ai::StimulusType::EventTimer,
            crate::ai::StimulusType::EventFitAgain,
        ],
        "both synchronous EventGotHit Thinks must preserve the genuinely pre-existing FIFO in exact order"
    );
    assert_eq!(
        victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .filter(|line| {
                line.line_type == crate::ai::LogLineType::Event
                    && line.info == crate::ai::StimulusType::EventGotHit as u16
            })
            .count(),
        2,
        "the direct retaliation and condolation EventGotHit handlers must both execute synchronously"
    );
    let sequence_count = engine.orders.sequence_manager.sequences_iter().count();
    engine.tick_ability_for(&sim, &mut display, &assets, attacker);
    assert_eq!(
        engine.orders.sequence_manager.sequences_iter().count(),
        sequence_count,
        "retaliation side effects must not repeat after terminal cleanup"
    );
}

#[test]
fn terminal_ability_owner_defers_exposed_generic_successor_until_next_hourglass() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    // This fixture isolates owner identity; the real projectile terminal
    // effect is covered by the production coordinator regression below.
    let mut element = SequenceElement::new(1, Command::Generic, Some(owner));
    let ability_order = Order::test_new(OrderType::ThrowingApple, 0.0, 0.0);
    let ability_id = ability_order.order_id;
    element.orders.push_back(ability_order);
    element
        .orders
        .push_back(Order::test_new(OrderType::WaitingUpright, 0.0, 0.0));
    let seq = engine.orders.sequence_manager.launch_element(element);
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_ability = crate::movement::ActiveAbility {
        kind: Some(crate::movement::AbilityKind::ThrowApple),
        sequence_id: Some(seq),
        element_index: 0,
        target: None,
        order_id: Some(ability_id),
        done_effect_applied: true,
        strangle_initialized: false,
    };
    let initial = engine
        .get_entity(owner)
        .unwrap()
        .element_data()
        .sprite
        .last_action;

    engine.tick_actor_animation_action_change_slots_with_hooks(
        &sim,
        &LevelAssets::new(),
        |_, _| {},
        |_, _| {},
        |engine, selected_owner, _, _, _, ability, _| {
            assert_eq!(
                (selected_owner, ability),
                (owner, Some((seq, 0, ability_id)))
            );
            engine
                .orders
                .sequence_manager
                .get_element_mut(seq, 0)
                .unwrap()
                .pop_current_order();
            engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap()
                .active_ability
                .clear();
        },
        |_, _, _| {},
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .element_data()
            .sprite
            .last_action,
        initial
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .unwrap()
            .2
            .order_type,
        OrderType::WaitingUpright
    );
}

#[test]
fn unbound_ability_catalog_order_still_uses_generic_execute() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let script = SpriteScript {
        action_id: OrderType::ThrowingApple as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::ThrowingApple as usize] = 0;
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    let mut element = SequenceElement::new(1, Command::Generic, Some(owner));
    element
        .orders
        .push_back(Order::test_new(OrderType::ThrowingApple, 0.0, 0.0));
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.tick_actor_animation_action_change_slots(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
    );

    assert_eq!(
        engine.get_entity(owner).unwrap().sprite().last_action,
        OrderType::ThrowingApple
    );
    assert!(
        !engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active()
    );
}

#[test]
fn ability_done_emits_once_retains_owner_and_only_terminated_releases() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    bind_test_action_point(
        &mut engine,
        owner,
        OrderType::Eating,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );
    {
        use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};
        let script = SpriteScript {
            action_id: OrderType::Eating as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2, 3],
            delays: vec![0, 0, 0],
            distances: vec![0, 0, 0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0; 3],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[OrderType::Eating as usize] = 0;
        engine
            .get_entity_mut(owner)
            .unwrap()
            .element_data_mut()
            .sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script]),
            std::sync::Arc::new(conversion),
        );
    }
    let element = SequenceElement::new(1, Command::EatCmd, Some(owner));
    let seq = engine.orders.sequence_manager.launch_element(element);
    assert_eq!(
        crate::abilities::begin_eat(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            owner,
            seq,
            0,
            &mut engine.orders.next_order_id
        ),
        crate::abilities::BeginResult::Started
    );
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    engine
        .orders
        .sequence_manager
        .get_element_mut(seq, 0)
        .unwrap()
        .orders
        .push_back(Order::test_new(OrderType::WaitingUpright, 0.0, 0.0));
    let order_id = engine
        .get_entity(owner)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_ability
        .order_id;
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let mut done_count = 0;
    loop {
        let results = crate::abilities::tick_ability(
            &sim,
            &mut engine.world.entities,
            &engine.orders.sequence_manager,
            owner,
            false,
        );
        done_count += results
            .iter()
            .filter(|result| matches!(result, crate::abilities::AbilityTickResult::EatDone { .. }))
            .count();
        if engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .done_effect_applied
        {
            break;
        }
    }
    assert_eq!(done_count, 1);
    let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
    assert_eq!(actor.active_ability.order_id, order_id);
    assert!(actor.active_ability.done_effect_applied);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .unwrap()
            .2
            .order_id,
        order_id.unwrap()
    );

    let mut display = CameraDisplayState::default();
    for _ in 0..10 {
        engine.tick_ability_for(&sim, &mut display, &assets, owner);
        if !engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active()
        {
            break;
        }
    }
    assert!(
        !engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active()
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .unwrap()
            .2
            .order_type,
        OrderType::WaitingUpright
    );
}

#[test]
fn unselected_listen_done_clears_action_without_dispatching_leave_listen() {
    use crate::element::{ActionState, Command, Posture};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    {
        let pc = engine.get_entity_mut(owner).unwrap();
        pc.actor_data_mut().unwrap().action_state = ActionState::Listening;
        pc.pc_data_mut().unwrap().current_action = crate::profiles::Action::Listen;
    }

    engine.apply_listen_done_action_handoff(owner);

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .pc_data()
            .unwrap()
            .current_action,
        crate::profiles::Action::NoAction
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .all(|element| element.command != Command::LeaveListen),
        "Original's unselected branch assigns the current action directly without deselecting it"
    );
}

#[test]
#[should_panic(expected = "is not a PC")]
fn listen_done_rejects_non_pc_owner() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));

    engine.apply_listen_done_action_handoff(owner);
}

#[test]
fn entity_slot_order_is_append_only_and_survives_save_round_trip() {
    let mut engine = EngineInner::new();
    let first = engine.add_entity(Entity::Scroll(crate::element::ElementScroll::default()));
    let second = engine.add_entity(Entity::Scroll(crate::element::ElementScroll::default()));

    engine.remove_entity(first);
    let third = engine.add_entity(Entity::Scroll(crate::element::ElementScroll::default()));

    assert_eq!(first.index(), 0);
    assert_eq!(second.index(), 1);
    assert_eq!(third.index(), 2, "removed slots must never be reused");
    assert_eq!(
        engine
            .world
            .entities
            .occupied()
            .map(|(id, _)| id.index())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );

    let encoded = serde_json::to_string(&engine.world.entities).expect("serialize entity slots");
    let decoded: crate::entities::Entities =
        serde_json::from_str(&encoded).expect("deserialize entity slots");
    assert_eq!(
        decoded
            .occupied()
            .map(|(id, _)| id.index())
            .collect::<Vec<_>>(),
        vec![1, 2],
        "save loading must preserve slot/creation order and holes"
    );
}

#[test]
fn hourglass_advances_mission_length_from_sim_seconds() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Campaign::new();

    for _ in 0..25 {
        let result = engine
            .perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev)
            .code;
        assert_eq!(result, GameCode::LevelInProgress);
    }

    assert_eq!(engine.control.frame_counter, 25);
    assert_eq!(
        engine
            .mission_domain
            .campaign
            .get_value(CampaignValue::MissionLength),
        1
    );
}

#[test]
fn enter_helping_climb_from_tree_retains_exit_prefix_until_animation_done() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let mut assets = LevelAssets::new();
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.characters.push(crate::profiles::CharacterProfile {
        actions: [
            crate::profiles::Action::HelpToClimb,
            crate::profiles::Action::NoAction,
            crate::profiles::Action::NoAction,
        ],
        ..Default::default()
    });
    assets.profile_manager = std::sync::Arc::new(profiles);
    let mut engine = EngineInner::new();

    let pc_id = engine.add_entity(crate::element::Entity::Pc(crate::element::ActorPc {
        element: {
            let mut initial_element =
                crate::element::ElementData::from_initial_posture(crate::element::Posture::Tree);
            initial_element.kind = crate::element::ElementKind::ActorPc;
            initial_element
        },
        actor: crate::element::ActorData {
            action_state: crate::element::ActionState::Waiting,
            ..Default::default()
        },
        human: Default::default(),
        pc: crate::element::PcData {
            life_points: 50,
            ..Default::default()
        },
    }));

    let elem = crate::sequence::SequenceElement::new(
        1,
        crate::element::Command::EnterHelpingClimb,
        Some(pc_id),
    );
    engine.launch_element(elem);
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let result = engine
        .perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev)
        .code;

    assert_eq!(result, GameCode::LevelInProgress);
    let pc = engine.get_entity(pc_id).expect("pc still exists");
    assert_eq!(
        pc.element_data().posture(),
        crate::element::Posture::Tree,
        "Translate must not apply the DONE-side posture early"
    );
    assert_eq!(
        pc.actor_data().unwrap().action_state,
        crate::element::ActionState::Waiting
    );
    let (sequence_id, element_index) = engine
        .orders
        .sequence_manager
        .current_element_for_actor(pc_id)
        .expect("helping-climb command remains selected while its animation runs");
    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence_id, element_index)
        .expect("selected helping-climb element still exists");
    assert_eq!(element.command, crate::element::Command::EnterHelpingClimb);
    assert_eq!(element.state, crate::sequence::SequenceState::InProgress);
    assert_eq!(
        element.current_order().map(|order| order.order_type),
        Some(crate::order::OrderType::TransitionWaitingHiddenWaitingUpright)
    );
}

#[test]
fn hourglass_quit_won() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.mission_domain.state.quit_won = true;
    let result = engine
        .perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev)
        .code;
    assert_eq!(result, GameCode::LevelSucceeded);
}

#[test]
fn hourglass_quit_lost() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.mission_domain.state.quit_lost = true;
    let result = engine
        .perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev)
        .code;
    assert_eq!(result, GameCode::LevelFailed);
}

#[test]
fn hourglass_quit_interrupted() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.mission_domain.state.quit_interrupted = true;
    let result = engine
        .perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev)
        .code;
    assert_eq!(result, GameCode::LevelInterrupted);
}

#[test]
fn hourglass_locked_skips_logic() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.set_engine_locked(true);
    // Even with a chorus timer, lock should prevent it from being decremented
    // (actually, chorus timer IS decremented before the lock check)
    engine.control.chorus_timer = 5;
    let result = engine
        .perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev)
        .code;
    assert_eq!(result, GameCode::LevelInProgress);
    // Chorus timer still decremented (it's before the lock check)
    assert_eq!(engine.control.chorus_timer, 4);
    // But frame counter is still incremented
    assert_eq!(engine.control.frame_counter, 1);
}

#[test]
fn explicit_quit_dispatch_unlinks_but_defers_state_change_to_lowering_start() {
    use crate::element::Command;
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;

    let sim = crate::sim_rng::test_context();
    let assets = assets_with_test_pc_profile();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let opponent = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

    {
        let owner_entity = engine.get_entity_mut(owner).unwrap();
        owner_entity.actor_data_mut().unwrap().action_state =
            crate::element::ActionState::WaitingSword;
        owner_entity.human_data_mut().unwrap().opponents = vec![opponent].into();
    }
    engine
        .get_entity_mut(opponent)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents = vec![owner].into();

    let sequence = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(
            1,
            Command::QuitSwordfight,
            Some(owner),
        ));
    engine.dispatch_quit_swordfight(&sim, &assets, owner, sequence, 0);
    // The InstructOwner dispatcher publishes the translated current order
    // through the actor's installed-order mirror right after the
    // per-command dispatch; mirror that boundary when calling the dispatch
    // arm directly.
    engine.publish_selected_order_for_instruct_owner(owner);

    assert!(
        engine
            .get_entity(owner)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents
            .is_empty()
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .action_state,
        crate::element::ActionState::WaitingSword,
        "translation must not switch to Waiting before lowering-sword START"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .and_then(|element| element.current_order())
            .map(|order| order.order_type),
        Some(OrderType::TransitionLoweringSword)
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .map(|element| element.state),
        Some(crate::sequence::SequenceState::InProgress),
        "QuitSwordfight translation must expose the command as current before lowering executes"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner),
        Some((sequence, 0))
    );
    assert_eq!(
        engine.actor_order_type(owner),
        Some(OrderType::TransitionLoweringSword),
        "accepted Instruct must publish the translated order through mpOrder"
    );
}
