use super::*;

use crate::element_kinds::Command;

fn tick_production_owner_coordinator(
    engine: &mut EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
) {
    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for (id, entity) in engine.world.entities.occupied() {
        positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
    }
    engine.tick_actor_owner_envelopes(sim, assets, &positions);
}

/// Run the movement phase and then the sequence-manager drain behind it.
///
/// A rider-charge hit does not damage its victim inside the rider's own
/// Execute: that arm only registers a `ReceiveSwordDamage` element, and
/// the manager Hourglass that follows the entity loop is what executes
/// it. Tests that want to observe the damage of a charge frame therefore
/// have to run both halves of the frame.
fn tick_movement_and_sequences(
    engine: &mut EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
) {
    engine.tick_entity_movement(sim, assets);
    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(sim, &mut display, assets);
}

#[test]
fn expired_failed_path_dispatches_owner_card_at_paths_barrier() {
    use crate::ai::{LogLineType, StimulusType};
    use crate::element::{Camp, Entity};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequenceState};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let earlier_timer_owner = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let expired_owner = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let nonexpired_owner = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let launch_failed_move = |engine: &mut EngineInner, owner| {
        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveWaiting,
            Some(owner),
            OrderType::WalkingUpright,
        );
        movement.orders.push_back(Order::new(
            OrderType::Freezing,
            0.0,
            0.0,
            engine.orders.allocate_order_id(),
        ));
        let sequence_id = engine.orders.sequence_manager.launch_element(movement);
        let _ = engine.orders.sequence_manager.hourglass();
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);
        sequence_id
    };
    let expired_sequence = launch_failed_move(&mut engine, expired_owner);
    let nonexpired_sequence = launch_failed_move(&mut engine, nonexpired_owner);
    engine.orders.failed_path_requests.extend([
        FailedPathRequest::from_pending(
            PendingPathRequest::test_request(expired_owner, expired_sequence, 0),
            0,
        ),
        FailedPathRequest::from_pending(
            PendingPathRequest::test_request(nonexpired_owner, nonexpired_sequence, 0),
            1,
        ),
    ]);
    engine.control.frame_counter = 101;
    let due_frame = engine.control.frame_counter;
    {
        let earlier_ai = engine
            .get_entity_mut(earlier_timer_owner)
            .and_then(Entity::ai_controller_mut)
            .expect("earlier timer owner has AI");
        earlier_ai.timer_is_running = true;
        earlier_ai.when_does_timer_ring = due_frame;
        earlier_ai.substate_at_last_timer_launch = earlier_ai.current_substate;
    }

    engine.hourglass_phase_paths(&sim, &assets);

    let expired = engine
        .orders
        .sequence_manager
        .get_element(expired_sequence, 0)
        .expect("expired movement remains registered through its owner card");
    assert_eq!(expired.state, SequenceState::Impossible);
    let expired_ai = engine
        .get_entity(expired_owner)
        .and_then(Entity::ai_controller)
        .expect("expired path owner retains AI");
    assert!(
        expired_ai.ai_log.iter().any(|line| {
            line.line_type == LogLineType::Event
                && line.info == StimulusType::EventCouldntReachPoint as u16
        }),
        "ProcessPathRequests must synchronously deliver the failed MOVE card before actor Hourglass slots"
    );

    let nonexpired = engine
        .orders
        .sequence_manager
        .get_element(nonexpired_sequence, 0)
        .expect("nonexpired movement remains registered");
    assert_eq!(nonexpired.state, SequenceState::InProgress);
    assert_eq!(engine.orders.failed_path_requests.len(), 1);
    assert_eq!(
        engine.orders.failed_path_requests[0].owner,
        nonexpired_owner
    );
    let nonexpired_ai = engine
        .get_entity(nonexpired_owner)
        .and_then(Entity::ai_controller)
        .expect("nonexpired path owner retains AI");
    assert!(
        !nonexpired_ai.ai_log.iter().any(|line| {
            line.line_type == LogLineType::Event
                && line.info == StimulusType::EventCouldntReachPoint as u16
        }),
        "a failure at age 100 must not dispatch early"
    );

    let earlier_ai = engine
        .get_entity(earlier_timer_owner)
        .and_then(Entity::ai_controller)
        .expect("earlier actor retains AI");
    assert!(
        earlier_ai.timer_is_running,
        "the due timer must remain armed until its owner Hourglass slot"
    );
    assert!(
        !earlier_ai.ai_log.iter().any(|line| {
            line.line_type == LogLineType::Event && line.info == StimulusType::EventTimer as u16
        }),
        "the paths barrier must finish before the earlier actor's timer slot begins"
    );

    engine.tick_ai_normal_timer_for_npc(&sim, earlier_timer_owner, &assets);
    let earlier_ai = engine
        .get_entity(earlier_timer_owner)
        .and_then(Entity::ai_controller)
        .expect("earlier actor retains AI after its timer slot");
    assert!(
        earlier_ai.ai_log.iter().any(|line| {
            line.line_type == LogLineType::Event && line.info == StimulusType::EventTimer as u16
        }),
        "the same due timer must dispatch when the earlier actor's timer slot runs"
    );
}

#[test]
fn make_fast_does_not_postprocess_an_unrelated_live_movement() {
    use crate::order::{Order, OrderType};
    use crate::sequence::{MoveFlags, Sequence, SequenceElement, SequenceElementData};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

    let mut selected = SequenceElement::new(1, Command::Generic, Some(owner));
    selected
        .orders
        .push_back(Order::test_new(OrderType::WaitingUprightBored, 0.0, 0.0));
    let selected_id = engine.orders.sequence_manager.launch_element(selected);
    engine
        .orders
        .sequence_manager
        .element_in_progress(selected_id, 0);

    let mut unrelated = Sequence::new();
    unrelated.append_element(SequenceElement::new_movement(
        1,
        Command::MoveWaiting,
        Some(owner),
        OrderType::WalkingUpright,
    ));
    let unrelated_id = engine.orders.sequence_manager.launch_sequence(unrelated);
    engine.orders.pending_path_requests =
        PendingPathRequestQueue::restore_v48_waiting(vec![PendingPathRequest::test_request(
            owner,
            unrelated_id,
            0,
        )]);

    engine.actor_make_fast(&crate::sim_rng::test_context(), owner);

    let SequenceElementData::Movement { action, flags, .. } = &engine
        .orders
        .sequence_manager
        .get_element(unrelated_id, 0)
        .expect("unrelated movement remains queued")
        .data
    else {
        panic!("movement variant");
    };
    assert_eq!(*action, OrderType::WalkingUpright);
    assert!(!flags.contains(MoveFlags::FAST));
    assert_eq!(
        engine.orders.pending_path_requests.v48_waiting()[0].move_action,
        OrderType::WalkingUpright,
        "Original tests only selected mpSequenceElement before invoking RHPathFinder::MakeFast"
    );
}

#[test]
fn menacing_ai_move_keeps_stop_menace_and_move_in_one_ordered_sequence() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .active = true;
    let mut intent =
        crate::order::AiOrderIntent::new(crate::order::OrderType::RunningUpright, 100.0, 200.0);
    intent.stop_menace_before_move = true;

    engine.orders.pending_move_requests.push((owner, intent));
    let sequence_id = engine
        .drain_pending_move_requests_for_owner(&crate::sim_rng::test_context(), owner)
        .into_iter()
        .next()
        .expect("same-sector AI move launches");
    let sequence = engine
        .orders
        .sequence_manager
        .get_sequence(sequence_id)
        .expect("movement sequence remains registered");

    assert_eq!(sequence.elements.len(), 2);
    assert_eq!(sequence.elements[0].command, Command::StopMenace);
    assert_eq!(sequence.elements[0].command_level, 1);
    assert_eq!(sequence.elements[1].command, Command::Move);
    assert_eq!(sequence.elements[1].command_level, 2);
}

#[test]
fn deferred_ai_move_builds_route_from_enqueue_time_topology() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let source_sector = crate::position_interface::SectorHandle::new(7);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.element_data_mut().active = true;
        entity
            .element_data_mut()
            .set_position_map(crate::coordinates::MapPoint::new(100.0, 200.0));
        entity.element_data_mut().set_sector(source_sector);
        entity.element_data_mut().set_layer(0);
    }

    let mut intent =
        crate::order::AiOrderIntent::new(crate::order::OrderType::RunningUpright, 400.0, 500.0);
    intent.target_sector = source_sector;
    intent.target_layer = Some(0);
    engine.launch_ai_move(owner, &intent);

    // A selected non-interruptible door element may commit its far-side
    // topology before SequenceManager gets to instruct the postponed GoTo.
    // Route construction must nevertheless use the topology at GoTo call
    // time, exactly as Original's synchronous AppendMoveToSequence does.
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity
            .element_data_mut()
            .set_position_map(crate::coordinates::MapPoint::new(125.0, 225.0));
        entity
            .element_data_mut()
            .set_sector(crate::position_interface::SectorHandle::new(77));
        entity.element_data_mut().set_layer(1);
    }

    let sequence_id = engine
        .drain_pending_move_requests_for_owner(&crate::sim_rng::test_context(), owner)
        .into_iter()
        .next()
        .expect("deferred same-sector route launches");
    let sequence = engine
        .orders
        .sequence_manager
        .get_sequence(sequence_id)
        .expect("movement sequence remains registered");

    assert_eq!(sequence.elements.len(), 1);
    assert_eq!(sequence.elements[0].command, Command::Move);
    let crate::sequence::SequenceElementData::Movement {
        destination,
        sector: target_sector,
        layer: target_layer,
        ..
    } = &sequence.elements[0].data
    else {
        panic!("call-time same-sector route must remain a movement element");
    };
    assert_eq!(
        *destination,
        crate::coordinates::MapPoint::new(400.0, 500.0)
    );
    assert_eq!(*target_sector, source_sector);
    assert_eq!(*target_layer, 0);
}

#[test]
fn production_owner_execution_frozen_blocks_rider_charge_execute_entirely() {
    use crate::engine::melee::{
        clear_test_sword_damage_observations, take_test_sword_damage_observations,
    };

    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let (rider, _, _) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RiderCharging,
        vec![0, 0],
    );
    engine
        .get_entity_mut(rider)
        .and_then(crate::element::Entity::enemy_ai_mut)
        .expect("rider has enemy AI")
        .hth_weapon_id = 1;
    add_charge_victim(&mut engine, MapPoint::new(100.0, 100.0));
    engine
        .get_entity_mut(rider)
        .and_then(crate::element::Entity::actor_data_mut)
        .expect("rider remains an actor")
        .execution_frozen = true;

    clear_test_sword_damage_observations();
    tick_production_owner_coordinator(&mut engine, &crate::sim_rng::test_context(), &assets);

    let actor = engine
        .get_entity(rider)
        .and_then(crate::element::Entity::actor_data)
        .expect("rider remains an actor");
    assert!(actor.active_rider_charge.is_none());
    assert!(actor.last_executed_rider_charge_order_id.is_none());
    assert!(take_test_sword_damage_observations().is_empty());
}

#[test]
fn frozen_all_repeats_running_rider_galopp_callback_on_the_frozen_frame() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let (rider, _, _) = install_galopp_fixture(&mut engine, &mut assets, vec![20, 20]);
    engine.set_actors_frozen(true);
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed = calls.clone();
    EngineInner::set_galopp_dispatch_observer(Some(Box::new(move |_, owner| {
        assert_eq!(owner, rider);
        observed.set(observed.get() + 1);
    })));

    for _ in 0..3 {
        tick_production_owner_coordinator(&mut engine, &crate::sim_rng::test_context(), &assets);
    }
    EngineInner::set_galopp_dispatch_observer(None);

    assert_eq!(calls.get(), 3);
    assert_eq!(engine.get_entity(rider).unwrap().sprite().current_frame, 0);
}

#[test]
fn one_frame_running_rider_fires_original_last_frame_galopp_disjunct() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let (rider, _, _) = install_galopp_fixture(&mut engine, &mut assets, vec![20]);
    engine.set_actors_frozen(true);
    let fired = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = fired.clone();
    EngineInner::set_galopp_dispatch_observer(Some(Box::new(move |_, owner| {
        assert_eq!(owner, rider);
        observed.set(true);
    })));
    tick_production_owner_coordinator(&mut engine, &crate::sim_rng::test_context(), &assets);
    EngineInner::set_galopp_dispatch_observer(None);
    assert!(fired.get());
}

#[test]
#[should_panic(expected = "selected RunningUpright rider-charge animation has no frames")]
fn selected_running_rider_galopp_requires_nonzero_animation_frames() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let (rider, _, _) = install_galopp_fixture(&mut engine, &mut assets, vec![20]);
    engine
        .get_entity_mut(rider)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::default();
    engine.set_actors_frozen(true);
    tick_production_owner_coordinator(&mut engine, &crate::sim_rng::test_context(), &assets);
}

#[test]
fn frozen_galopp_think_closes_before_movement_completion_and_next_owner_slot() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let (rider, sequence, order_id) = install_galopp_fixture(&mut engine, &mut assets, vec![20]);
    let later = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    engine.set_actors_frozen(true);
    let callback_closed = std::rc::Rc::new(std::cell::Cell::new(false));
    let callback_observed = callback_closed.clone();
    EngineInner::set_galopp_dispatch_observer(Some(Box::new(move |engine, owner| {
        assert_eq!(owner, rider);
        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("frozen gallop retains its selected movement element");
        assert_eq!(element.state, crate::sequence::SequenceState::InProgress);
        assert_eq!(element.current_order().unwrap().order_id, order_id);
        callback_observed.set(true);
    })));

    let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    let next_owner_observed = std::rc::Rc::new(std::cell::Cell::new(false));
    let next_observed = next_owner_observed.clone();
    let callback_for_next = callback_closed.clone();
    engine.tick_actor_owner_envelopes_with_test_owner_hook(
        &crate::sim_rng::test_context(),
        &assets,
        &positions,
        move |_, owner| {
            if owner == later {
                assert!(
                    callback_for_next.get(),
                    "gallop Think/script/order drain must close before the next owner slot"
                );
                next_observed.set(true);
            }
        },
    );
    EngineInner::set_galopp_dispatch_observer(None);
    assert!(callback_closed.get());
    assert!(next_owner_observed.get());
}

#[test]
#[should_panic(expected = "GALOPP Execute callback owner Soldier(SoldierId(0)) is not a rider")]
fn galopp_execute_callback_rejects_non_rider_owner() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
    engine.dispatch_galopp_loop_event(&crate::sim_rng::test_context(), &LevelAssets::new(), owner);
}

#[test]
#[should_panic(expected = "disappeared before its synchronous GALOPP Execute callback")]
fn galopp_execute_callback_rejects_missing_selected_owner() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
    engine.remove_entity(owner);
    engine.dispatch_galopp_loop_event(&crate::sim_rng::test_context(), &LevelAssets::new(), owner);
}

#[test]
fn production_owner_uses_exact_selected_element_not_background_movement() {
    use crate::element::Command;
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let (rider, selected_seq, _) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        OrderType::RiderCharging,
        vec![0, 0],
    );
    engine
        .get_entity_mut(rider)
        .and_then(crate::element::Entity::enemy_ai_mut)
        .expect("rider has enemy AI")
        .hth_weapon_id = 1;

    let mut background =
        SequenceElement::new_movement(1, Command::Move, Some(rider), OrderType::RiderCharging);
    background
        .orders
        .push_back(Order::test_new(OrderType::RiderCharging, 300.0, 100.0));
    let movement_seq = engine.orders.sequence_manager.launch_element(background);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_seq, 0);

    let generic_data = SequenceElement::new_generic(1, Command::Point, Some(rider)).data;
    let selected = engine
        .orders
        .sequence_manager
        .get_element_mut(selected_seq, 0)
        .expect("selected fixture element remains installed");
    selected.command = Command::Point;
    selected.data = generic_data;
    // Production Point elements always carry the pointing direction; the
    // animation dispatcher reads it to drive the direction goal.
    selected.set_property(
        crate::sequence::Field::Direction,
        crate::sequence::FieldValue::Integer(0),
    );
    selected.orders.clear();
    selected
        .orders
        .push_back(Order::test_new(OrderType::WaitingUpright, 0.0, 0.0));
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(rider),
        Some((selected_seq, 0))
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(movement_seq, 0)
            .expect("background movement remains installed")
            .state,
        crate::sequence::SequenceState::InProgress
    );

    engine.set_actors_frozen(true);
    tick_production_owner_coordinator(&mut engine, &crate::sim_rng::test_context(), &assets);
    let actor = engine
        .get_entity(rider)
        .and_then(crate::element::Entity::actor_data)
        .expect("rider remains an actor");
    assert!(actor.active_rider_charge.is_none());
    assert!(actor.last_executed_rider_charge_order_id.is_none());
}

fn install_rider_charge_fixture(
    engine: &mut EngineInner,
    assets: &mut LevelAssets,
    order_type: crate::order::OrderType,
    frame_delays: Vec<u16>,
) -> (EntityId, crate::sequence::SequenceId, std::num::NonZeroU32) {
    use crate::element::{ActionState, Camp, Command, Entity, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::Order;
    use crate::profiles::{CharacterProfile, HtHWeaponProfile, SoldierProfile};
    use crate::sequence::{MoveFlags, SequenceElement};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    profiles.soldiers.push(SoldierProfile {
        hth_weapon_id: 1,
        rider: true,
        ..SoldierProfile::default()
    });
    let mut weapon = HtHWeaponProfile::default();
    weapon.thrusts[crate::weapons::SwordStrike::Charge as usize].cutting = 9;
    weapon.thrusts[crate::weapons::SwordStrike::Charge as usize].repulsion = 30;
    profiles.hth_weapons.push(weapon);
    profiles.characters.push(CharacterProfile {
        hth_weapon_id: 1,
        endurance: 50,
        ..CharacterProfile::default()
    });

    let mut rider = make_test_ai_soldier(Camp::Royalists);
    let Entity::Soldier(soldier) = &mut rider else {
        unreachable!()
    };
    soldier.soldier.rider = true;
    soldier.soldier.soldier_profile_index = crate::profiles::SoldierProfileIdx(0);
    soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("rider fixture remains enemy AI")
        .hth_weapon_id = 1;
    soldier.element.active = true;
    soldier.element.posture = Posture::Upright;
    soldier
        .element
        .set_position_map(MapPoint::new(100.0, 100.0));
    soldier.element.set_direction_instantly(0);
    soldier.actor.action_state = ActionState::MovingFast;

    let frames = frame_delays.len();
    let make_script = |action| SpriteScript {
        action_id: action as u16,
        action_done: frames.saturating_sub(1) as u16,
        average_speed: 4.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: frames as u16 * 4,
        frame_ids: (0..frames as u32).collect(),
        delays: frame_delays.clone(),
        distances: vec![4; frames],
        offsets: vec![SpriteFrameOffset::ZERO; frames],
        sound_ids: vec![0; frames],
    };
    let transition = make_script(crate::order::OrderType::TransitionCharging);
    let running = make_script(crate::order::OrderType::RunningUpright);
    let mut scripts = vec![transition; 16];
    scripts.extend(vec![running; 16]);
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[crate::order::OrderType::TransitionCharging as usize] = 0;
    conversion[crate::order::OrderType::RunningUpright as usize] = 16;
    soldier.element.sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(scripts),
        std::sync::Arc::new(conversion),
    );
    soldier
        .element
        .sprite
        .position_iface
        .set_anti_collision_on(false);
    soldier
        .element
        .set_position_map(MapPoint::new(100.0, 100.0));
    let rider_id = engine.add_entity(rider);

    let order_id = engine.orders.allocate_order_id();
    let mut order = Order::new(order_type, 300.0, 100.0, order_id);
    order.compute_direction = true;
    order.tolerance = 7.0;
    order.lock_ai = true;
    order.move_flags = MoveFlags::RIDER_CHARGE.bits() as u16;
    let mut movement = SequenceElement::new_movement(1, Command::Move, Some(rider_id), order_type);
    if let crate::sequence::SequenceElementData::Movement { flags, .. } = &mut movement.data {
        *flags = MoveFlags::RIDER_CHARGE;
    }
    movement.orders.push_back(order);
    let sequence_id = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);
    engine
        .get_entity_mut(rider_id)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_movement = ActiveMovement::new(sequence_id, 0);
    (rider_id, sequence_id, order_id)
}

fn install_galopp_fixture(
    engine: &mut EngineInner,
    assets: &mut LevelAssets,
    frame_delays: Vec<u16>,
) -> (EntityId, crate::sequence::SequenceId, std::num::NonZeroU32) {
    use crate::order::OrderType;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let result =
        install_rider_charge_fixture(engine, assets, OrderType::RunningUpright, vec![20, 20]);
    let frames = frame_delays.len();
    let script = SpriteScript {
        action_id: OrderType::RunningUpright as u16,
        action_done: frames.saturating_sub(1) as u16,
        average_speed: 4.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: frames as u16 * 4,
        frame_ids: (0..frames as u32).collect(),
        delays: frame_delays,
        distances: vec![4; frames],
        offsets: vec![SpriteFrameOffset::ZERO; frames],
        sound_ids: vec![0; frames],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::RunningUpright as usize] = 0;
    let rider = engine.get_entity_mut(result.0).unwrap();
    rider
        .enemy_ai_mut()
        .expect("gallop fixture remains an enemy soldier")
        .hth_weapon_id = 1;
    rider.element_data_mut().sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    rider
        .element_data_mut()
        .sprite
        .position_iface
        .set_anti_collision_on(false);
    result
}

fn rider_charge_point(origin: MapPoint, direction: i16, forward: f32, side: f32) -> MapPoint {
    let [forward_x, forward_y] = crate::position_interface::sector_to_vector_iso(direction);
    let [side_x, side_y] = crate::position_interface::sector_to_vector_iso((direction + 4) & 15);
    MapPoint::new(
        origin.x + forward * forward_x + side * side_x,
        origin.y + forward * forward_y + side * side_y,
    )
}

fn add_charge_victim(engine: &mut EngineInner, position: MapPoint) -> EntityId {
    let mut victim = make_test_pc(crate::element::Posture::Upright);
    victim.element_data_mut().active = true;
    victim.element_data_mut().set_position_map(position);
    victim
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_coords(
            -4.0, -4.0, 4.0, 4.0,
        ));
    engine.add_entity(victim)
}

fn install_charge_victim_motion(
    engine: &mut EngineInner,
    victim_id: EntityId,
    start: MapPoint,
    goal: MapPoint,
) {
    use crate::element::{ActionState, Command};
    use crate::movement::ActiveMovement;
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let action = OrderType::WalkingUpright;
    let script = SpriteScript {
        action_id: action as u16,
        action_done: 0,
        average_speed: 10.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 10,
        frame_ids: vec![1],
        delays: vec![20],
        distances: vec![10],
        offsets: vec![SpriteFrameOffset::ZERO],
        sound_ids: vec![0],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[action as usize] = 0;
    let entity = engine.get_entity_mut(victim_id).unwrap();
    entity.element_data_mut().sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    entity
        .element_data_mut()
        .sprite
        .position_iface
        .set_anti_collision_on(false);
    entity
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_coords(
            -4.0, -4.0, 4.0, 4.0,
        ));
    entity.element_data_mut().set_position_map(start);
    entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

    let order_id = engine.orders.allocate_order_id();
    let mut movement = SequenceElement::new_movement(1, Command::Move, Some(victim_id), action);
    movement
        .orders
        .push_back(Order::new(action, goal.x, goal.y, order_id));
    let sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    engine
        .get_entity_mut(victim_id)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_movement = ActiveMovement::new(sequence, 0);
}

#[test]
fn production_owner_final_arrival_drains_reachpoint_condolation_exactly_once() {
    use crate::ai::StimulusType;
    use crate::element::{Command, Posture};
    use crate::engine::soldier_helpers::{
        capture_condolation_stimuli, capture_owner_boundary_resumes,
        install_condolation_nested_termination,
    };
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let mut mover = make_test_pc(Posture::Upright);
    mover.element_data_mut().active = true;
    mover.element_data_mut().posture = Posture::Upright;
    let mover_id = engine.add_entity(mover);
    install_charge_victim_motion(
        &mut engine,
        mover_id,
        MapPoint::new(0.0, 0.0),
        MapPoint::new(1.0, 0.0),
    );
    engine
        .get_entity_mut(mover_id)
        .expect("mover remains installed")
        .element_data_mut()
        .sprite
        .last_processed_order_id = u32::MAX;
    let movement_seq = engine
        .get_entity(mover_id)
        .and_then(crate::element::Entity::actor_data)
        .and_then(|actor| actor.active_movement.sequence_id)
        .expect("movement is armed");

    let foreign_owner = engine.add_entity(make_test_pc(Posture::Upright));
    let nested_owner = engine.add_entity(make_test_pc(Posture::Upright));
    let foreign_seq = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::Wait, Some(foreign_owner)));
    engine
        .orders
        .sequence_manager
        .element_in_progress(foreign_seq, 0);
    engine
        .orders
        .sequence_manager
        .element_terminated(foreign_seq, 0);
    let nested_seq = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::Wait, Some(nested_owner)));
    engine
        .orders
        .sequence_manager
        .element_in_progress(nested_seq, 0);
    install_condolation_nested_termination(mover_id, StimulusType::EventReachPoint, nested_seq, 0);
    let sim = crate::sim_rng::test_context();

    let ((_, resumes), trace) = capture_condolation_stimuli(|| {
        capture_owner_boundary_resumes(|| {
            tick_production_owner_coordinator(&mut engine, &sim, &assets)
        })
    });

    // The foreign owner's pre-existing card is not stolen by the mover's
    // boundary; it drains at that owner's own actor slot later in the same
    // tick, mirroring the synchronous condolence-card delivery of SetState.
    assert_eq!(
        trace,
        vec![
            (mover_id, StimulusType::EventReachPoint),
            (nested_owner, StimulusType::EventDone),
            (foreign_owner, StimulusType::EventDone),
        ]
    );
    assert_eq!(
        resumes,
        vec![nested_owner, mover_id, foreign_owner],
        "the nested cross-owner SetState must close before A resumes Ready/successors"
    );
    assert!(engine.orders.timer_elements.is_empty());
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(movement_seq, 0)
            .expect("completed movement remains inspectable")
            .state,
        crate::sequence::SequenceState::Terminated
    );
    let backlog = engine.orders.sequence_manager.drain_pending_condolations();
    assert!(
        backlog.is_empty(),
        "every owner's condolation drains at its own boundary within the tick"
    );
}

#[test]
fn rider_charge_approach_never_initializes_from_flags_alone() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let (rider, _, _) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RunningUpright,
        vec![0, 0, 0],
    );

    engine.tick_entity_movement(&crate::sim_rng::test_context(), &assets);

    assert!(
        engine
            .get_entity(rider)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_rider_charge
            .is_none(),
        "RIDER_CHARGE on RunningUpright is only the approach/gallop loop"
    );
}

#[test]
fn rider_charging_action_executes_without_rider_charge_flag() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let (rider, sequence, _) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RiderCharging,
        vec![20, 1],
    );
    let element = engine
        .orders
        .sequence_manager
        .get_element_mut(sequence, 0)
        .unwrap();
    if let crate::sequence::SequenceElementData::Movement { flags, .. } = &mut element.data {
        *flags = crate::sequence::MoveFlags::empty();
    }
    element.orders.front_mut().unwrap().move_flags = 0;

    engine.tick_entity_movement(&crate::sim_rng::test_context(), &assets);

    assert!(
        engine
            .get_entity(rider)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_rider_charge
            .is_some(),
        "RiderCharging dispatch is keyed solely by the live action"
    );
}

#[test]
fn rider_charge_fresh_id_same_action_replacement_reinitializes_candidates_immediately() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let origin = MapPoint::new(100.0, 100.0);
    let (rider, sequence, old_order_id) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RiderCharging,
        vec![20, 20],
    );
    let stale = add_charge_victim(&mut engine, rider_charge_point(origin, 0, 80.0, 30.0));
    let replacement = add_charge_victim(&mut engine, MapPoint::new(900.0, 900.0));
    let sim = crate::sim_rng::test_context();

    engine.tick_entity_movement(&sim, &assets);
    assert_eq!(
        engine
            .get_entity(rider)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_rider_charge
            .as_ref()
            .unwrap()
            .pending_victims,
        vec![stale]
    );

    engine
        .get_entity_mut(stale)
        .unwrap()
        .element_data_mut()
        .set_position_map(MapPoint::new(900.0, 900.0));
    engine
        .get_entity_mut(replacement)
        .unwrap()
        .element_data_mut()
        .set_position_map(rider_charge_point(origin, 0, 100.0, 30.0));
    let replacement_order_id = engine.orders.allocate_order_id();
    assert_ne!(replacement_order_id, old_order_id);
    engine
        .orders
        .sequence_manager
        .get_element_mut(sequence, 0)
        .unwrap()
        .orders
        .front_mut()
        .unwrap()
        .order_id = replacement_order_id;

    engine.tick_entity_movement(&sim, &assets);

    assert_eq!(
        engine
            .get_entity(rider)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_rider_charge
            .as_ref()
            .unwrap()
            .pending_victims,
        vec![replacement],
        "fresh same-action identity clears and rebuilds candidates before motion"
    );
}

#[test]
fn rider_charge_uses_actual_sprite_waits_and_rewrites_same_order_on_actual_last_frame() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let (rider, sequence, old_id) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RiderCharging,
        vec![2, 1, 1],
    );
    let sim = crate::sim_rng::test_context();

    let mut observed_frames = Vec::new();
    for _ in 0..6 {
        engine.tick_entity_movement(&sim, &assets);
        observed_frames.push(engine.get_entity(rider).unwrap().sprite().current_frame);
    }
    assert_eq!(observed_frames, vec![0, 0, 0, 1, 1, 2]);
    let rewritten = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap()
        .current_order()
        .unwrap();
    assert_eq!(
        rewritten.order_type,
        crate::order::OrderType::RunningUpright
    );
    assert_ne!(rewritten.order_id, old_id);
    assert_eq!(rewritten.tolerance, 7.0);
    assert!(rewritten.lock_ai);
    assert!(rewritten.compute_direction);
    assert!(
        engine
            .get_entity(rider)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_rider_charge
            .is_none()
    );
}

#[test]
fn rider_charge_initializes_once_resamples_geometry_and_keeps_wrong_layer_pending() {
    use crate::element::Posture;
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let (rider, _, _) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RiderCharging,
        vec![20, 1, 1],
    );
    let mut victim = make_test_pc(Posture::Upright);
    victim.element_data_mut().active = true;
    let [forward_x, forward_y] = crate::position_interface::sector_to_vector_iso(0);
    let [side_x, side_y] = crate::position_interface::sector_to_vector_iso(4);
    victim.element_data_mut().set_position_map(MapPoint::new(
        100.0 + 100.0 * forward_x + 30.0 * side_x,
        100.0 + 100.0 * forward_y + 30.0 * side_y,
    ));
    let victim_id = engine.add_entity(victim);
    let sim = crate::sim_rng::test_context();

    engine.tick_entity_movement(&sim, &assets);
    assert_eq!(
        engine
            .get_entity(rider)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_rider_charge
            .as_ref()
            .unwrap()
            .pending_victims,
        vec![victim_id]
    );

    // Move both participants after initialization. The second polygon must
    // use the rider's new sample, while eligibility must not be rerun.
    engine
        .get_entity_mut(rider)
        .unwrap()
        .element_data_mut()
        .set_position_map(MapPoint::new(200.0, 100.0));
    engine
        .get_entity_mut(victim_id)
        .unwrap()
        .element_data_mut()
        .set_layer(1);
    engine.tick_entity_movement(&sim, &assets);
    assert_eq!(
        engine
            .get_entity(rider)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_rider_charge
            .as_ref()
            .unwrap()
            .pending_victims,
        vec![victim_id],
        "a candidate on another live layer stays pending"
    );
}

#[test]
fn rider_charge_interruption_clears_state_and_new_charge_reinitializes() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let (rider, sequence, _) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RiderCharging,
        vec![20, 1],
    );
    let sim = crate::sim_rng::test_context();
    engine.tick_entity_movement(&sim, &assets);
    assert!(
        engine
            .get_entity(rider)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_rider_charge
            .is_some()
    );

    let interrupted = engine
        .orders
        .sequence_manager
        .get_element_mut(sequence, 0)
        .unwrap();
    interrupted.orders.front_mut().unwrap().order_type = crate::order::OrderType::RunningUpright;
    let crate::sequence::SequenceElementData::Movement { flags, .. } = &mut interrupted.data else {
        panic!("rider charge fixture must remain a movement element");
    };
    *flags = crate::sequence::MoveFlags::empty();
    engine.set_actors_frozen(true);
    engine.tick_entity_movement(&sim, &assets);
    engine.set_actors_frozen(false);
    assert!(
        engine
            .get_entity(rider)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_rider_charge
            .is_none()
    );

    let fresh_id = engine.orders.allocate_order_id();
    let order = engine
        .orders
        .sequence_manager
        .get_element_mut(sequence, 0)
        .unwrap()
        .orders
        .front_mut()
        .unwrap();
    order.order_type = crate::order::OrderType::RiderCharging;
    order.order_id = fresh_id;
    engine.tick_entity_movement(&sim, &assets);
    assert!(
        engine
            .get_entity(rider)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_rider_charge
            .is_some()
    );
}

#[test]
fn rider_charge_frozen_all_still_initializes_and_runs_polygon_on_frozen_frame() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let (rider, _, _) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RiderCharging,
        vec![3, 1],
    );
    engine.set_actors_frozen(true);

    engine.tick_entity_movement(&crate::sim_rng::test_context(), &assets);

    let entity = engine.get_entity(rider).unwrap();
    assert_eq!(entity.sprite().current_frame, 0);
    assert_eq!(
        entity.element_data().position_map(),
        MapPoint::new(100.0, 100.0)
    );
    assert!(
        entity.actor_data().unwrap().active_rider_charge.is_some(),
        "FrozenAll preserves ExecuteRiderCharge initialization/polygon work"
    );
}

#[test]
fn rider_charge_frozen_all_real_victim_is_damaged_once_across_multiple_ticks() {
    use crate::engine::melee::{
        clear_test_sword_damage_observations, take_test_sword_damage_observations,
    };
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let origin = MapPoint::new(100.0, 100.0);
    let (rider, _, order_id) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RiderCharging,
        vec![3, 1],
    );
    let victim = add_charge_victim(&mut engine, rider_charge_point(origin, 0, 0.0, 30.0));
    engine.set_actors_frozen(true);
    let sim = crate::sim_rng::test_context();
    clear_test_sword_damage_observations();

    tick_movement_and_sequences(&mut engine, &sim, &assets);
    tick_movement_and_sequences(&mut engine, &sim, &assets);
    tick_movement_and_sequences(&mut engine, &sim, &assets);

    let observations = take_test_sword_damage_observations();
    assert_eq!(observations.len(), 1, "frozen ticks must not rebuild hits");
    assert_eq!(observations[0].victim_id, victim);
    assert!(observations[0].life_points_after > 0, "victim is nonlethal");
    let entity = engine.get_entity(rider).unwrap();
    assert_eq!(entity.sprite().current_frame, 0);
    assert_eq!(entity.sprite().last_processed_order_id, u32::MAX);
    assert_eq!(
        entity
            .actor_data()
            .unwrap()
            .last_executed_rider_charge_order_id,
        Some(order_id)
    );
    assert!(
        entity
            .actor_data()
            .unwrap()
            .active_rider_charge
            .as_ref()
            .unwrap()
            .pending_victims
            .is_empty(),
        "the resolved victim remains removed while FrozenAll persists"
    );
}

#[test]
fn rider_charge_frozen_all_fresh_id_same_action_reinitializes_candidates() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let origin = MapPoint::new(100.0, 100.0);
    let (rider, sequence, old_order_id) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RiderCharging,
        vec![3, 1],
    );
    let stale = add_charge_victim(&mut engine, rider_charge_point(origin, 0, 80.0, 30.0));
    let replacement = add_charge_victim(&mut engine, MapPoint::new(900.0, 900.0));
    engine.set_actors_frozen(true);
    let sim = crate::sim_rng::test_context();

    engine.tick_entity_movement(&sim, &assets);
    assert_eq!(
        engine
            .get_entity(rider)
            .unwrap()
            .actor_data()
            .unwrap()
            .last_executed_rider_charge_order_id,
        Some(old_order_id)
    );
    assert_eq!(
        engine
            .get_entity(rider)
            .unwrap()
            .sprite()
            .last_processed_order_id,
        u32::MAX
    );
    assert_eq!(
        engine
            .get_entity(rider)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_rider_charge
            .as_ref()
            .unwrap()
            .pending_victims,
        vec![stale]
    );

    engine
        .get_entity_mut(stale)
        .unwrap()
        .element_data_mut()
        .set_position_map(MapPoint::new(900.0, 900.0));
    engine
        .get_entity_mut(replacement)
        .unwrap()
        .element_data_mut()
        .set_position_map(rider_charge_point(origin, 0, 100.0, 30.0));
    let fresh_id = engine.orders.allocate_order_id();
    engine
        .orders
        .sequence_manager
        .get_element_mut(sequence, 0)
        .unwrap()
        .orders
        .front_mut()
        .unwrap()
        .order_id = fresh_id;

    engine.tick_entity_movement(&sim, &assets);

    let entity = engine.get_entity(rider).unwrap();
    assert_eq!(entity.sprite().last_processed_order_id, u32::MAX);
    assert_eq!(
        entity
            .actor_data()
            .unwrap()
            .last_executed_rider_charge_order_id,
        Some(fresh_id)
    );
    assert_eq!(
        entity
            .actor_data()
            .unwrap()
            .active_rider_charge
            .as_ref()
            .unwrap()
            .pending_victims,
        vec![replacement],
        "fresh frozen identity rebuilds candidates without a noncharge tick"
    );
}

#[test]
fn rider_charge_frozen_then_unfrozen_initializes_sprite_motion_on_first_live_tick() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let (rider, _, order_id) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RiderCharging,
        vec![3, 1],
    );
    let sim = crate::sim_rng::test_context();
    engine.set_actors_frozen(true);

    engine.tick_entity_movement(&sim, &assets);
    engine.tick_entity_movement(&sim, &assets);
    {
        let entity = engine.get_entity(rider).unwrap();
        assert_eq!(entity.sprite().last_processed_order_id, u32::MAX);
        assert_eq!(
            entity
                .actor_data()
                .unwrap()
                .last_executed_rider_charge_order_id,
            Some(order_id)
        );
    }

    engine.set_actors_frozen(false);
    engine.tick_entity_movement(&sim, &assets);

    let entity = engine.get_entity(rider).unwrap();
    assert_eq!(entity.sprite().last_processed_order_id, order_id.get());
    assert_eq!(
        entity.sprite().last_action,
        crate::order::OrderType::TransitionCharging
    );
    assert_eq!(
        entity.sprite().position_iface.map_goal(),
        MapPoint::new(300.0, 100.0)
    );
    assert!(entity.sprite().position_iface.is_increment_map_computed());
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        crate::element::ActionState::MovingFast,
        "the first unfrozen PerformMotion reports Start"
    );
}

#[test]
fn rider_charge_real_hit_lands_once_only_and_uses_post_turn_flight_direction() {
    use crate::engine::melee::{
        clear_test_sword_damage_observations, take_test_sword_damage_observations,
    };
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let origin = MapPoint::new(100.0, 100.0);
    // Put the victim before the rider in creation order. On the frame after
    // damage translation, its FallingHit Execute must sample the rider's
    // post-hit direction before the rider takes another charge Turn step.
    let victim = add_charge_victim(&mut engine, rider_charge_point(origin, 0, 0.0, 30.0));
    let (rider, _, _) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RiderCharging,
        vec![20, 1],
    );
    // Frame zero's polygon is the one-sided width segment at the pre-Turn
    // origin. The fixture's eastward goal turns the live rider from 0 to 1.
    clear_test_sword_damage_observations();

    tick_movement_and_sequences(&mut engine, &crate::sim_rng::test_context(), &assets);

    let observations = take_test_sword_damage_observations();
    assert_eq!(observations.len(), 1);
    let hit = &observations[0];
    assert_eq!((hit.attacker_id, hit.victim_id), (rider, victim));
    assert_eq!(hit.strike, crate::weapons::SwordStrike::Charge);
    assert!(hit.active_rider_charge);
    assert!(
        hit.pending_victims.is_empty(),
        "the hit leaves the pending list when its damage element is registered"
    );
    assert!(
        hit.life_points_after < hit.life_points_before,
        "the manager drain behind the entity loop applies the damage"
    );
    assert_eq!(
        hit.attacker_direction, 1,
        "flight samples live post-Turn facing"
    );
    let [flight_x, flight_y] = crate::position_interface::sector_to_vector_iso(1);
    let expected_facing =
        (crate::position_interface::vector_to_sector_0_to_15(flight_x, flight_y) + 8) & 15;
    assert_eq!(
        hit.victim_direction_after, 0,
        "TranslateHitDamage only authors the fall order; ReadyForTakeOff is deferred to Execute"
    );
    assert!(
        engine
            .get_entity(rider)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_rider_charge
            .as_ref()
            .unwrap()
            .pending_victims
            .is_empty()
    );

    clear_test_sword_damage_observations();
    // The first frame's manager drain only translates ReceiveSwordDamage and
    // installs FallingHit. ReadyForTakeOff belongs to that order's next
    // source-authored owner Execute slot, not the movement-only fixture path.
    tick_production_owner_coordinator(&mut engine, &crate::sim_rng::test_context(), &assets);
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .element_data()
            .direction(),
        expected_facing,
        "the fall order's first Execute faces the victim opposite live rider direction 1"
    );
    assert!(
        take_test_sword_damage_observations().is_empty(),
        "victim is hit once"
    );
}

#[test]
fn rider_charge_multiple_hits_follow_creation_order_rng_state_and_holes() {
    use crate::engine::melee::{
        clear_test_sword_damage_observations, take_test_sword_damage_observations,
    };

    fn run() -> Vec<(u32, i16, i16, Vec<u32>)> {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let origin = MapPoint::new(100.0, 100.0);
        let (_rider, _, _) = install_rider_charge_fixture(
            &mut engine,
            &mut assets,
            crate::order::OrderType::RiderCharging,
            vec![20, 1],
        );
        let first = add_charge_victim(&mut engine, rider_charge_point(origin, 0, 0.0, 20.0));
        let hole = add_charge_victim(&mut engine, MapPoint::new(900.0, 900.0));
        let second = add_charge_victim(&mut engine, rider_charge_point(origin, 0, 0.0, 40.0));
        engine.remove_entity(hole);
        clear_test_sword_damage_observations();

        tick_movement_and_sequences(&mut engine, &crate::sim_rng::test_context(), &assets);

        let observations = take_test_sword_damage_observations();
        assert_eq!(
            observations
                .iter()
                .map(|hit| hit.victim_id)
                .collect::<Vec<_>>(),
            vec![first, second],
            "candidate order follows live creation slots across a hole"
        );
        // The charge frame drains the whole pending list while it registers
        // the damage elements, so neither hit still sees a candidate waiting
        // by the time the manager drain executes them.
        assert!(observations[0].pending_victims.is_empty());
        assert!(observations[1].pending_victims.is_empty());
        observations
            .into_iter()
            .map(|hit| {
                (
                    hit.victim_id.index(),
                    hit.life_points_before,
                    hit.life_points_after,
                    hit.pending_victims
                        .into_iter()
                        .map(EntityId::index)
                        .collect(),
                )
            })
            .collect()
    }

    assert_eq!(
        run(),
        run(),
        "damage/RNG and state mutation order is deterministic"
    );
}

#[test]
fn rider_charge_last_frame_damage_lands_after_the_rewrite_and_clear() {
    use crate::engine::melee::{
        clear_test_sword_damage_observations, take_test_sword_damage_observations,
    };
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let origin = MapPoint::new(100.0, 100.0);
    let (rider, sequence, old_id) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RiderCharging,
        vec![0],
    );
    add_charge_victim(&mut engine, rider_charge_point(origin, 0, 10.0, 30.0));
    clear_test_sword_damage_observations();

    tick_movement_and_sequences(&mut engine, &crate::sim_rng::test_context(), &assets);

    let observations = take_test_sword_damage_observations();
    assert_eq!(observations.len(), 1);
    // The rider's own Execute rewrites its order and drops the charge on the
    // last frame; the damage element it registered only runs afterwards, in
    // the manager drain, so it never observes a live charge.
    assert!(!observations[0].active_rider_charge);
    let order = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap()
        .current_order()
        .unwrap();
    assert_eq!(order.order_type, crate::order::OrderType::RunningUpright);
    assert_ne!(order.order_id, old_id);
    assert!(
        engine
            .get_entity(rider)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_rider_charge
            .is_none()
    );
}

#[test]
fn rider_charge_initial_eligibility_is_not_rechecked_and_returning_layer_can_hit() {
    use crate::engine::melee::{
        clear_test_sword_damage_observations, take_test_sword_damage_observations,
    };
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let (rider, _, _) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RiderCharging,
        vec![20, 20],
    );
    let victim = add_charge_victim(
        &mut engine,
        rider_charge_point(MapPoint::new(100.0, 100.0), 0, 80.0, 30.0),
    );
    let sim = crate::sim_rng::test_context();
    clear_test_sword_damage_observations();
    tick_movement_and_sequences(&mut engine, &sim, &assets);
    assert!(take_test_sword_damage_observations().is_empty());

    // Eligibility changes after initialization are intentionally ignored.
    // Wrong layer merely postpones geometry; returning to the sampled layer
    // later permits the already-retained candidate to hit.
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .active = false;
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_layer(1);
    tick_movement_and_sequences(&mut engine, &sim, &assets);
    assert!(take_test_sword_damage_observations().is_empty());
    let rider_origin = engine
        .get_entity(rider)
        .unwrap()
        .element_data()
        .position_map();
    let rider_direction = engine.get_entity(rider).unwrap().element_data().direction();
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_position_map(rider_charge_point(rider_origin, rider_direction, 0.0, 30.0));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_layer(0);
    // Activeness is restored with the layer: retention is what this test is
    // about, but the receive-sword-damage dispatch independently refuses an
    // inactive owner, so an off-world victim would never reach the rolls.
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .active = true;
    tick_movement_and_sequences(&mut engine, &sim, &assets);
    assert_eq!(take_test_sword_damage_observations().len(), 1);
}

#[test]
fn rider_charge_owner_slot_sees_earlier_movement_and_interrupts_later_before_movement() {
    use crate::engine::melee::{
        clear_test_sword_damage_observations, take_test_sword_damage_observations,
    };
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let origin = MapPoint::new(100.0, 100.0);

    // Reserve the earlier creation slot before installing the rider.
    let earlier = add_charge_victim(&mut engine, MapPoint::new(100.0, 100.0));
    let (rider, _, _) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RiderCharging,
        vec![1, 1],
    );
    let hit_point = rider_charge_point(origin, 1, 0.0, 30.0);
    let [move_x, move_y] = crate::position_interface::sector_to_vector_iso(4);
    let earlier_start = MapPoint::new(hit_point.x - 6.0 * move_x, hit_point.y - 6.0 * move_y);
    let earlier_goal = MapPoint::new(hit_point.x + 20.0 * move_x, hit_point.y + 20.0 * move_y);
    install_charge_victim_motion(&mut engine, earlier, earlier_start, earlier_goal);

    let later = add_charge_victim(&mut engine, hit_point);
    let later_goal = MapPoint::new(hit_point.x + 10.0 * move_x, hit_point.y + 10.0 * move_y);
    install_charge_victim_motion(&mut engine, later, hit_point, later_goal);
    engine
        .get_entity_mut(later)
        .unwrap()
        .sprite_mut()
        .frame_count = 1;
    assert!(earlier.index() < rider.index() && rider.index() < later.index());
    let sim = crate::sim_rng::test_context();
    clear_test_sword_damage_observations();

    // First tick initializes all three motions and the charge candidates.
    // Ordinary Start frames now correctly commit their nonzero distance: the
    // earlier victim enters the charge polygon before the rider samples it.
    // Hold the later victim on a non-distance wait counter, then restore both
    // positions on each animation wait tick until the rider reaches its
    // actual charge decision frame.
    tick_movement_and_sequences(&mut engine, &sim, &assets);
    assert!(take_test_sword_damage_observations().is_empty());
    let observations = (0..4)
        .find_map(|_| {
            let earlier_entity = engine.get_entity_mut(earlier).unwrap();
            earlier_entity
                .element_data_mut()
                .set_position_map(earlier_start);
            earlier_entity.sprite_mut().frame_count = 0;
            let later_entity = engine.get_entity_mut(later).unwrap();
            later_entity.element_data_mut().set_position_map(hit_point);
            later_entity.sprite_mut().frame_count = 1;
            clear_test_sword_damage_observations();
            tick_movement_and_sequences(&mut engine, &sim, &assets);
            let observations = take_test_sword_damage_observations();
            (!observations.is_empty()).then_some(observations)
        })
        .expect("rider must reach its charge decision frame");
    assert_eq!(
        observations
            .iter()
            .map(|hit| hit.victim_id)
            .collect::<Vec<_>>(),
        vec![earlier, later]
    );
    assert_eq!(
        engine
            .get_entity(earlier)
            .unwrap()
            .element_data()
            .position_map(),
        earlier_start,
        "the rider interrupts the earlier victim on its no-distance wait frame"
    );
    assert_eq!(
        engine
            .get_entity(later)
            .unwrap()
            .element_data()
            .position_map(),
        hit_point,
        "later victim is damaged/interrupted before its movement slot"
    );
}

#[test]
#[should_panic(expected = "missing TransitionCharging animation")]
fn rider_charge_requires_transition_animation() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let (rider, _, _) = install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RiderCharging,
        vec![1],
    );
    engine
        .get_entity_mut(rider)
        .unwrap()
        .element_data_mut()
        .sprite
        .conversion = std::sync::Arc::new(vec![
        crate::sprite_script::UNMAPPED;
        crate::sprite_script::NONANIMATION_END
    ]);
    engine.tick_entity_movement(&crate::sim_rng::test_context(), &assets);
}

#[test]
#[should_panic(expected = "missing hand-to-hand weapon profile")]
fn rider_charge_requires_weapon_profile() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    install_rider_charge_fixture(
        &mut engine,
        &mut assets,
        crate::order::OrderType::RiderCharging,
        vec![1],
    );
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .hth_weapons
        .clear();
    engine.tick_entity_movement(&crate::sim_rng::test_context(), &assets);
}

#[test]
fn current_movement_bootstraps_from_waiting_with_destination_state() {
    use crate::element::{ActionState, Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite::MotionOrderContext;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let start = MapPoint::new(100.0, 100.0);
    let destination = MapPoint::new(140.0, 100.0);
    let mut mover = make_test_pc(Posture::Upright);
    mover.element_data_mut().active = true;
    mover.element_data_mut().set_position_map(start);
    let mover_id = engine.add_entity(mover);

    let action = OrderType::WalkingUpright;
    let script = SpriteScript {
        action_id: action as u16,
        action_done: 0,
        average_speed: 20.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 20,
        frame_ids: vec![1],
        delays: vec![0],
        distances: vec![20],
        offsets: vec![SpriteFrameOffset::ZERO],
        sound_ids: vec![0],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[action as usize] = 0;
    let mut sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    sprite.position_iface.set_anti_collision_on(false);
    engine
        .get_entity_mut(mover_id)
        .expect("movement fixture actor exists")
        .element_data_mut()
        .sprite = sprite;
    engine
        .get_entity_mut(mover_id)
        .unwrap()
        .element_data_mut()
        .set_position_map(start);

    let order_id = engine.orders.allocate_order_id();
    let order = Order::new(action, destination.x, destination.y, order_id);
    let mut movement = SequenceElement::new_movement(1, Command::Move, Some(mover_id), action);
    movement.orders.push_back(order);
    let sequence_id = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);
    engine
        .get_entity_mut(mover_id)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_movement = ActiveMovement::new(sequence_id, 0);

    assert_eq!(
        engine
            .get_entity(mover_id)
            .unwrap()
            .actor_data()
            .unwrap()
            .action_state,
        ActionState::Waiting
    );
    engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());

    let entity = engine.get_entity(mover_id).unwrap();
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::Moving,
        "the first PerformMotion tick must enter the walking state"
    );
    assert_eq!(
        entity.element_data().position_map(),
        MapPoint::new(112.0, 100.0),
        "the Start invocation still commits its nonzero PerformMotion frame"
    );
    assert_eq!(
        entity
            .element_data()
            .sprite
            .motion_order_state_mismatch(MotionOrderContext {
                order_id,
                destination,
                reverse: false,
                tolerance: 0.0,
                directional_tolerance: false,
                compute_direction: false,
                next_destination_same_action: None,
                target_element: None,
            }),
        None,
        "the first movement tick must seed the order's destination instead of generic action state"
    );
}

#[test]
fn move_waiting_freeze_does_not_enter_destination_motion() {
    use crate::element::{ActionState, Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let position = MapPoint::new(1352.0, 246.0);
    let mut mover = make_test_pc(Posture::Upright);
    mover.element_data_mut().active = true;
    mover.element_data_mut().set_position_map(position);
    mover.actor_data_mut().unwrap().action_state = ActionState::Moving;
    let mover_id = engine.add_entity(mover);

    let preserved_action = OrderType::TransitionWaitingUprightRunningUpright;
    let script = SpriteScript {
        action_id: preserved_action as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2],
        delays: vec![2, 2],
        distances: vec![0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
        sound_ids: vec![0; 2],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[preserved_action as usize] = 0;
    conversion[OrderType::WaitingUpright as usize] = 16;
    let mut sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 32]),
        std::sync::Arc::new(conversion),
    );
    sprite.last_action = preserved_action;
    sprite.current_row = 0;
    sprite.current_frame = 1;
    sprite.frame_count = 1;
    engine
        .get_entity_mut(mover_id)
        .unwrap()
        .element_data_mut()
        .sprite = sprite;
    engine
        .get_entity_mut(mover_id)
        .unwrap()
        .element_data_mut()
        .set_position_map(position);

    let order_id = engine.orders.allocate_order_id();
    let mut movement = SequenceElement::new_movement(
        1,
        Command::MoveWaiting,
        Some(mover_id),
        OrderType::WalkingUpright,
    );
    movement.orders.push_back(Order::new(
        OrderType::Freezing,
        position.x,
        position.y,
        order_id,
    ));
    let sequence_id = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);
    engine
        .get_entity_mut(mover_id)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_movement = ActiveMovement::new(sequence_id, 0);

    engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());
    engine.tick_actor_animation_action_change_slots(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
    );

    let entity = engine.get_entity(mover_id).unwrap();
    assert_eq!(entity.element_data().position_map(), position);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::Moving
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .unwrap()
            .current_order()
            .unwrap()
            .order_type,
        OrderType::Freezing,
        "MOVE_WAITING must retain its pathfinder hold order"
    );
    let sprite = &entity.element_data().sprite;
    assert_eq!(sprite.last_action, preserved_action);
    assert_eq!(sprite.current_row, 0);
    assert_eq!(sprite.current_frame, 1);
    assert_eq!(
        sprite.frame_count, 1,
        "FREEZING must not select, stamp, or advance any sprite animation"
    );
}

#[test]
fn npc_follow_observes_target_position_at_its_creation_order_boundary() {
    #[derive(Debug, PartialEq)]
    struct Observation {
        frame: u32,
        observer_slot: u32,
        target_slot: u32,
        target_before_movement: MapPoint,
        target_after_movement: MapPoint,
        target_position_observed_by_follow: crate::coordinates::GroundPoint,
    }

    fn observe(observer_before_target: bool) -> Observation {
        use crate::element::{Camp, Entity, Posture};

        let mut engine = EngineInner::new();
        engine.control.frame_counter = 73;

        let mut observer = make_test_ai_soldier(Camp::Lacklandists);
        observer.element_data_mut().active = true;
        observer
            .element_data_mut()
            .set_position_map(MapPoint::new(100.0, 100.0));
        observer.element_data_mut().set_direction_instantly(0);

        let mut target = make_test_pc(Posture::Upright);
        target.element_data_mut().active = true;
        let target_before_movement = MapPoint::new(80.0, 20.0);
        let target_after_movement = MapPoint::new(120.0, 20.0);
        target
            .element_data_mut()
            .set_position_map(target_before_movement);
        target.position_iface_mut().set_obstacle(
            None,
            Some(crate::position_interface::PlaneZCoeffs {
                az: 0.0,
                bz: 0.0,
                dz: 45.0,
            }),
        );

        let (observer_id, target_id) = if observer_before_target {
            let observer_id = engine.add_entity(observer);
            let target_id = engine.add_entity(target);
            (observer_id, target_id)
        } else {
            let target_id = engine.add_entity(target);
            let observer_id = engine.add_entity(observer);
            (observer_id, target_id)
        };

        let mut positions_before_movement =
            crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        for (entity_id, entity) in engine.world.entities.occupied() {
            positions_before_movement[entity_id] =
                Some(crate::entities::BoundaryPosition::of(entity.element_data()));
        }

        // This mutation is the smallest deterministic stand-in for the
        // globally batched tick_entity_movement between the captured input
        // boundary and refresh_npc_views. The oracle is the position copied
        // into EYES_FOLLOW's stare point, not movement-distance mechanics.
        engine
            .get_entity_mut(target_id)
            .expect("follow target exists")
            .element_data_mut()
            .set_position_map(target_after_movement);
        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("follow observer exists")
        else {
            panic!("follow observer changed entity kind");
        };
        crate::ai_vision::focus_entity(&mut observer.npc, target_id);

        engine.refresh_npc_views(&positions_before_movement);

        let Entity::Soldier(observer) = engine
            .get_entity(observer_id)
            .expect("follow observer remains")
        else {
            panic!("follow observer changed entity kind");
        };
        Observation {
            frame: engine.control.frame_counter,
            observer_slot: observer_id.index(),
            target_slot: target_id.index(),
            target_before_movement,
            target_after_movement,
            target_position_observed_by_follow: observer.npc.stare_point,
        }
    }

    assert_eq!(
        [observe(true), observe(false)],
        [
            Observation {
                frame: 73,
                observer_slot: 0,
                target_slot: 1,
                target_before_movement: MapPoint::new(80.0, 20.0),
                target_after_movement: MapPoint::new(120.0, 20.0),
                target_position_observed_by_follow: crate::coordinates::GroundPoint::new(
                    80.0, 65.0,
                ),
            },
            Observation {
                frame: 73,
                observer_slot: 1,
                target_slot: 0,
                target_before_movement: MapPoint::new(80.0, 20.0),
                target_after_movement: MapPoint::new(120.0, 20.0),
                target_position_observed_by_follow: crate::coordinates::GroundPoint::new(
                    120.0, 65.0,
                ),
            },
        ],
        "original per-element virtual calls expose pre-move state to an earlier observer and post-move state to a later observer"
    );
}

#[test]
fn seek_tolerance_observes_target_position_at_its_creation_order_boundary() {
    use crate::element::{ActionState, Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{Order, OrderType};
    use crate::position_interface::{Direction, SectorHandle};
    use crate::sequence::{MoveFlags, SequenceElement, SequenceElementData, SequenceState};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    #[derive(Debug, PartialEq)]
    struct Observation {
        seeker_slot: u32,
        target_slot: u32,
        target_before_movement: MapPoint,
        target_after_movement: MapPoint,
        seeker_after_crossing_tolerance: MapPoint,
        seeker_direction_after_crossing_tolerance: i16,
        seeker_state_after_crossing_tolerance: SequenceState,
        seeker_after_next_tolerance_sample: MapPoint,
        seeker_state_after_next_tolerance_sample: SequenceState,
    }

    fn bind_walking_sprite(engine: &mut EngineInner, entity_id: EntityId) {
        let action = OrderType::WalkingUpright;
        let script = SpriteScript {
            action_id: action as u16,
            action_done: 0,
            average_speed: 20.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 20,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![20],
            offsets: vec![SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[action as usize] = 0;
        let mut sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );

        let element = engine
            .get_entity_mut(entity_id)
            .expect("movement fixture actor exists")
            .element_data_mut();
        let position = element.position_map();
        let sector = element.sector();
        sprite.position_iface.set_sector(sector);
        sprite.position_iface.set_anti_collision_on(false);
        sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                MapVec::new(-2.0, -2.0),
                MapVec::new(2.0, 2.0),
            ));
        element.sprite = sprite;
        element.set_position_map(position);
    }

    fn arm_movement(
        engine: &mut EngineInner,
        owner: EntityId,
        destination: MapPoint,
        seek_target: Option<EntityId>,
    ) -> crate::sequence::SequenceId {
        let mut element =
            SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
        element.orders.push_back(Order::test_new(
            OrderType::WalkingUpright,
            destination.x,
            destination.y,
        ));
        let SequenceElementData::Movement {
            destination: element_destination,
            sector,
            element: element_target,
            flags,
            tolerance,
            ..
        } = &mut element.data
        else {
            unreachable!("new_movement must create movement data")
        };
        *element_destination = destination;
        *sector = SectorHandle::new(1);
        *element_target = seek_target;
        if seek_target.is_some() {
            *flags = MoveFlags::SEEK;
            *tolerance = 15.0;
        }

        let sequence_id = engine.orders.sequence_manager.launch_element(element);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);
        let actor = engine
            .get_entity_mut(owner)
            .expect("movement owner exists")
            .actor_data_mut()
            .expect("movement owner is an actor");
        actor.action_state = ActionState::Moving;
        actor.active_movement = ActiveMovement::new(sequence_id, 0);
        if seek_target.is_some() {
            // The live-target arrival test uses the actor-owned seek
            // distance (the unadapted interaction radius), not the
            // movement element's path tolerance.
            actor.seek_distance = 15.0;
        }
        sequence_id
    }

    fn observe(seeker_before_target: bool) -> Observation {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = EngineInner::new();
        let target_before_movement = MapPoint::new(10.0, 0.0);
        let target_destination = MapPoint::new(30.0, 0.0);

        let mut seeker = make_test_pc(Posture::Upright);
        seeker.element_data_mut().active = true;
        seeker
            .element_data_mut()
            .set_position_map(MapPoint::new(0.0, 0.0));
        seeker.element_data_mut().set_sector(SectorHandle::new(1));

        let mut target = make_test_pc(Posture::Upright);
        target.element_data_mut().active = true;
        target
            .element_data_mut()
            .set_position_map(target_before_movement);
        target.element_data_mut().set_sector(SectorHandle::new(1));

        let (seeker_id, target_id) = if seeker_before_target {
            let seeker_id = engine.add_entity(seeker);
            let target_id = engine.add_entity(target);
            (seeker_id, target_id)
        } else {
            let target_id = engine.add_entity(target);
            let seeker_id = engine.add_entity(seeker);
            (seeker_id, target_id)
        };

        bind_walking_sprite(&mut engine, seeker_id);
        bind_walking_sprite(&mut engine, target_id);
        arm_movement(&mut engine, target_id, target_destination, None);
        let seeker_sequence = arm_movement(
            &mut engine,
            seeker_id,
            MapPoint::new(100.0, 0.0),
            Some(target_id),
        );
        if seeker_before_target {
            engine
                .orders
                .sequence_manager
                .get_element_mut(seeker_sequence, 0)
                .unwrap()
                .orders[0]
                .compute_direction = false;
            let position = engine
                .get_entity_mut(seeker_id)
                .unwrap()
                .position_iface_mut();
            position.set_direction_instantly(Direction::NORTH);
            position.set_direction(Direction::EAST);
            position.deviated = true;
            let _ = position.turn();
            let _ = position.turn();
        }

        // The original sprite pipeline reports MotionState::Start without
        // advancing on a newly-seen order. Prime that start tick, then use
        // the next production movement tick as the ordering observation.
        let assets = LevelAssets::new();
        engine.tick_entity_movement(sim, &assets);
        engine.tick_entity_movement(sim, &assets);

        let seeker_after_crossing_tolerance = engine
            .get_entity(seeker_id)
            .expect("seeker remains after crossing tolerance")
            .element_data()
            .position_map();
        let seeker_state_after_crossing_tolerance = engine
            .orders
            .sequence_manager
            .get_element(seeker_sequence, 0)
            .expect("seeker movement remains after crossing tolerance")
            .state;

        // Entity-target PerformSeek does not re-sample tolerance after its
        // committed step. The next actor tick observes the now-in-range
        // position and terminates without another movement commit.
        engine.tick_entity_movement(sim, &assets);

        Observation {
            seeker_slot: seeker_id.index(),
            target_slot: target_id.index(),
            target_before_movement,
            target_after_movement: engine
                .get_entity(target_id)
                .expect("target remains after movement")
                .element_data()
                .position_map(),
            seeker_after_crossing_tolerance,
            seeker_direction_after_crossing_tolerance: engine
                .get_entity(seeker_id)
                .unwrap()
                .element_data()
                .direction(),
            seeker_state_after_crossing_tolerance,
            seeker_after_next_tolerance_sample: engine
                .get_entity(seeker_id)
                .expect("seeker remains after movement")
                .element_data()
                .position_map(),
            seeker_state_after_next_tolerance_sample: engine
                .orders
                .sequence_manager
                .get_element(seeker_sequence, 0)
                .expect("seeker movement element remains inspectable")
                .state,
        }
    }

    assert_eq!(
        [observe(true), observe(false)],
        [
            Observation {
                seeker_slot: 0,
                target_slot: 1,
                target_before_movement: MapPoint::new(10.0, 0.0),
                target_after_movement: MapPoint::new(30.0, 0.0),
                // This seeker observes the target's pre-movement position,
                // which is already within 15 units, and terminates without
                // committing a step.
                seeker_after_crossing_tolerance: MapPoint::new(0.0, 0.0),
                seeker_direction_after_crossing_tolerance: 0,
                seeker_state_after_crossing_tolerance: SequenceState::Terminated,
                seeker_after_next_tolerance_sample: MapPoint::new(0.0, 0.0),
                seeker_state_after_next_tolerance_sample: SequenceState::Terminated,
            },
            Observation {
                seeker_slot: 1,
                target_slot: 0,
                target_before_movement: MapPoint::new(10.0, 0.0),
                target_after_movement: MapPoint::new(30.0, 0.0),
                // This seeker observes the target after its movement. Two
                // 12-unit turning-slowed frames cross into tolerance, but
                // the second frame remains in progress until the next
                // pre-motion sample.
                seeker_after_crossing_tolerance: MapPoint::new(24.0, 0.0),
                seeker_direction_after_crossing_tolerance: 1,
                seeker_state_after_crossing_tolerance: SequenceState::InProgress,
                seeker_after_next_tolerance_sample: MapPoint::new(24.0, 0.0),
                seeker_state_after_next_tolerance_sample: SequenceState::Terminated,
            },
        ],
        "seek tolerance uses the target position visible at the actor boundary and is not re-sampled after a committed step"
    );
}

#[test]
fn final_arrival_step_runs_actor_anti_collision_before_snapping() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::element::{ActionState, Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{Order, OrderType};
    use crate::position_interface::SectorHandle;
    use crate::sequence::{SequenceElement, SequenceElementData, SequenceState};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    fn bind_walking_sprite(engine: &mut EngineInner, entity_id: EntityId) {
        let action = OrderType::WalkingUpright;
        let script = SpriteScript {
            action_id: action as u16,
            action_done: 0,
            average_speed: 20.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 20,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![20],
            offsets: vec![SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[action as usize] = 0;
        let mut sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );

        let element = engine
            .get_entity_mut(entity_id)
            .expect("anti-collision fixture actor exists")
            .element_data_mut();
        let position = element.position_map();
        let sector = element.sector();
        sprite.position_iface.set_sector(sector);
        sprite.position_iface.set_anti_collision_on(true);
        sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                MapVec::new(-2.0, -2.0),
                MapVec::new(2.0, 2.0),
            ));
        element.sprite = sprite;
        element.set_position_map(position);
    }

    let mut engine = EngineInner::new();
    let destination = MapPoint::new(10.0, 0.0);

    let mut mover = make_test_pc(Posture::Upright);
    mover.element_data_mut().active = true;
    mover
        .element_data_mut()
        .set_position_map(MapPoint::new(0.0, 0.0));
    mover.element_data_mut().set_sector(SectorHandle::new(1));

    let mut blocker = make_test_pc(Posture::Upright);
    blocker.element_data_mut().active = true;
    blocker.element_data_mut().set_position_map(destination);
    blocker.element_data_mut().set_sector(SectorHandle::new(1));

    let mover_id = engine.add_entity(mover);
    let blocker_id = engine.add_entity(blocker);
    bind_walking_sprite(&mut engine, mover_id);
    bind_walking_sprite(&mut engine, blocker_id);

    let mut movement =
        SequenceElement::new_movement(1, Command::Move, Some(mover_id), OrderType::WalkingUpright);
    movement.orders.push_back(Order::test_new(
        OrderType::WalkingUpright,
        destination.x,
        destination.y,
    ));
    let SequenceElementData::Movement {
        destination: movement_destination,
        sector,
        ..
    } = &mut movement.data
    else {
        unreachable!("new_movement must create movement data")
    };
    *movement_destination = destination;
    *sector = SectorHandle::new(1);

    let sequence_id = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);
    let actor = engine
        .get_entity_mut(mover_id)
        .expect("mover exists")
        .actor_data_mut()
        .expect("mover is an actor");
    actor.action_state = ActionState::Moving;
    actor.active_movement = ActiveMovement::new(sequence_id, 0);

    // A newly-seen motion order spends one tick in MotionState::Start.
    // On the next tick the destination is within one animation step. The
    // original game still applies actor repulsion before checking arrival.
    let assets = LevelAssets::new();
    engine.tick_entity_movement(sim, &assets);
    engine.tick_entity_movement(sim, &assets);

    let mover_position = engine
        .get_entity(mover_id)
        .expect("mover remains after movement")
        .element_data()
        .position_map();
    let blocker_position = engine
        .get_entity(blocker_id)
        .expect("blocker remains after movement")
        .element_data()
        .position_map();
    assert_ne!(
        mover_position, blocker_position,
        "the final movement tick must not snap the mover onto another actor"
    );
    // The deflected step leaves the mover deviated and blocked in place.
    // The deviated + blocked arm of the goal-reached check accepts any
    // position whose max-norm distance to the goal is under 10 units, so
    // the movement terminates where the repulsion left the actor instead
    // of snapping onto the goal (and the blocker standing on it).
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .expect("movement remains inspectable")
            .state,
        SequenceState::Terminated,
        "a blocked deflected final step arrives in place without the goal snap"
    );
}

#[test]
fn deviated_blocked_post_step_arrival_pops_intermediate_waypoint_without_snapping() {
    use crate::element::{ActionState, Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{Order, OrderType};
    use crate::position_interface::SectorHandle;
    use crate::sequence::{SequenceElement, SequenceElementData, SequenceState};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let start = MapPoint::new(0.0, 0.0);
    let intermediate = MapPoint::new(14.0, 0.0);
    let final_goal = MapPoint::new(100.0, 0.0);

    let mut mover = make_test_pc(Posture::Upright);
    mover.element_data_mut().active = true;
    mover.element_data_mut().set_position_map(start);
    mover.element_data_mut().set_sector(SectorHandle::new(1));
    let mover_id = engine.add_entity(mover);

    let action = OrderType::WalkingUpright;
    let script = SpriteScript {
        action_id: action as u16,
        action_done: 0,
        average_speed: 10.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 10,
        frame_ids: vec![1],
        delays: vec![0],
        distances: vec![10],
        offsets: vec![SpriteFrameOffset::ZERO],
        sound_ids: vec![0],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[action as usize] = 0;
    let mut sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    // Keep this fixture focused on PerformMotion ordering: the actor starts
    // the committed frame in the state produced by a prior blocked
    // anti-collision attempt, while the next anti-collision call commits the
    // ordinary step unchanged.
    sprite.position_iface.set_sector(SectorHandle::new(1));
    sprite.position_iface.set_anti_collision_on(false);
    sprite
        .position_iface
        .set_move_box(crate::coordinates::MoveBox::from_coords(
            -2.0, -2.0, 2.0, 2.0,
        ));
    engine
        .get_entity_mut(mover_id)
        .unwrap()
        .element_data_mut()
        .sprite = sprite;
    engine
        .get_entity_mut(mover_id)
        .unwrap()
        .element_data_mut()
        .set_position_map(start);

    let mut movement = SequenceElement::new_movement(1, Command::Move, Some(mover_id), action);
    movement
        .orders
        .push_back(Order::test_new(action, intermediate.x, intermediate.y));
    movement
        .orders
        .push_back(Order::test_new(action, final_goal.x, final_goal.y));
    let SequenceElementData::Movement {
        destination,
        sector,
        ..
    } = &mut movement.data
    else {
        unreachable!("new_movement must create movement data")
    };
    *destination = final_goal;
    *sector = SectorHandle::new(1);

    let sequence_id = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);
    let actor = engine
        .get_entity_mut(mover_id)
        .unwrap()
        .actor_data_mut()
        .unwrap();
    actor.action_state = ActionState::Moving;
    actor.active_movement = ActiveMovement::new(sequence_id, 0);

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    engine.tick_entity_movement(&sim, &assets);
    {
        let pi = engine
            .get_entity_mut(mover_id)
            .unwrap()
            .position_iface_mut();
        pi.deviated = true;
        pi.blocked_count = 1;
    }
    engine.tick_entity_movement(&sim, &assets);

    let mover = engine.get_entity(mover_id).unwrap();
    assert_eq!(
        mover.element_data().position_map(),
        MapPoint::new(12.0, 0.0),
        "the actor starts this frame 8 units away, commits its 6-unit turn-slowed step, and remains unsnapped while deviated"
    );
    let movement = engine
        .orders
        .sequence_manager
        .get_element(sequence_id, 0)
        .unwrap();
    assert_eq!(movement.state, SequenceState::InProgress);
    assert_eq!(movement.orders.len(), 1);
    let current = movement.current_order().unwrap();
    assert_eq!(
        MapPoint::new(current.target_x, current.target_y),
        final_goal,
        "post-step IsGoalReached must pop exactly the intermediate waypoint"
    );
}

#[test]
fn npc_hourglass_uses_exact_wrapped_register_frame_phase() {
    use super::ai::npc_hourglass_frame_phase;

    let sixteenth_frame_visits: Vec<_> = (0..256)
        .filter_map(|frame| {
            let phase = npc_hourglass_frame_phase(frame, 0);
            (phase & 15 == 0).then_some((frame, phase))
        })
        .collect();
    assert_eq!(
        sixteenth_frame_visits,
        vec![
            (4, 160),
            (20, 176),
            (36, 192),
            (52, 208),
            (68, 224),
            (84, 240),
            (100, 0),
            (116, 16),
            (132, 32),
            (148, 48),
            (164, 64),
            (180, 80),
            (196, 96),
            (212, 112),
            (228, 128),
            (244, 144),
        ]
    );
    assert_eq!(
        sixteenth_frame_visits
            .iter()
            .filter_map(|&(frame, phase)| (phase & 63 == 0).then_some(frame))
            .collect::<Vec<_>>(),
        vec![36, 100, 164, 228]
    );
}

#[test]
fn npc_hourglass_tail_drains_old_lock_queue_only_after_unlock() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let soldier_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let ai = engine
        .get_entity_mut(soldier_id)
        .and_then(|entity| entity.ai_controller_mut())
        .expect("test soldier has AI");
    ai.locks_flag_field = crate::ai::AiLockFlags::BUSY;
    ai.stimulus_queue.push(crate::ai::Stimulus::new(
        crate::ai::StimulusType::EventAfterCombatInjury,
    ));

    engine.tick_ai_queued_stimuli(sim, &assets);
    assert_eq!(
        engine
            .get_entity(soldier_id)
            .and_then(|entity| entity.ai_controller())
            .unwrap()
            .stimulus_queue
            .len(),
        1,
        "the Hourglass lock gate must preserve queued stimuli"
    );

    engine
        .get_entity_mut(soldier_id)
        .and_then(|entity| entity.ai_controller_mut())
        .unwrap()
        .locks_flag_field = crate::ai::AiLockFlags::empty();
    engine.tick_ai_queued_stimuli(sim, &assets);
    assert!(
        engine
            .get_entity(soldier_id)
            .and_then(|entity| entity.ai_controller())
            .unwrap()
            .stimulus_queue
            .is_empty(),
        "the final unlocked Hourglass phase must replay the old lock queue"
    );
}

fn install_seen_enemy(engine: &mut EngineInner, npc_id: EntityId, target: EntityId) {
    use crate::element::{Detectable, DetectableType};

    engine
        .get_entity_mut(npc_id)
        .and_then(|entity| entity.npc_data_mut())
        .expect("NPC has data")
        .detectable_lists[DetectableType::Enemy as usize] = vec![Detectable {
        element: Some(target),
        detectable_type: DetectableType::Enemy,
        seen_now: true,
        seen_last_frame: true,
        ..Detectable::default()
    }];
}

fn enemy_blink_state(engine: &EngineInner, npc_id: EntityId) -> (bool, bool) {
    use crate::element::DetectableType;

    let detectable = &engine
        .get_entity(npc_id)
        .and_then(|entity| entity.npc_data())
        .expect("NPC has data")
        .detectable_lists[DetectableType::Enemy as usize][0];
    (detectable.seen_now, detectable.seen_last_frame)
}

#[test]
fn deferred_wakeup_pc_applies_specific_blink_inline_to_opposite_camp_npcs() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::combat::ConcussionOutcome;
    use crate::element::{Camp, Posture};

    let mut engine = EngineInner::new();
    let waker = engine.add_entity(make_test_pc(Posture::Upright));
    let same_camp_npc = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let opposite_camp_npc = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    install_seen_enemy(&mut engine, same_camp_npc, waker);
    install_seen_enemy(&mut engine, opposite_camp_npc, waker);

    engine
        .orders
        .pending_concussion_side_effects
        .push((waker, ConcussionOutcome::WokeUp));
    engine.drain_pending_concussion_side_effects(sim, &LevelAssets::new());

    assert_eq!(enemy_blink_state(&engine, same_camp_npc), (true, true));
    assert_eq!(
        enemy_blink_state(&engine, opposite_camp_npc),
        (false, false)
    );
}

#[test]
fn deferred_wakeup_soldier_defers_blink_until_its_creation_slot() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::combat::ConcussionOutcome;
    use crate::element::Camp;

    let mut engine = EngineInner::new();
    engine.ai.global.there_are_royalist_soldiers = true;
    engine.ai.global.there_are_lacklandist_soldiers = true;
    let waker = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let same_camp_npc = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let opposite_camp_npc = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    install_seen_enemy(&mut engine, same_camp_npc, waker);
    install_seen_enemy(&mut engine, opposite_camp_npc, waker);

    engine
        .orders
        .pending_concussion_side_effects
        .push((waker, ConcussionOutcome::WokeUp));
    engine.drain_pending_concussion_side_effects(sim, &LevelAssets::new());

    assert_eq!(enemy_blink_state(&engine, same_camp_npc), (true, true));
    assert_eq!(enemy_blink_state(&engine, opposite_camp_npc), (true, true));
    assert!(
        engine
            .get_entity(waker)
            .and_then(|entity| entity.ai_controller())
            .unwrap()
            .outbox
            .detection
            .stimuli
            .iter()
            .any(|stimulus| stimulus.stimulus_type == crate::ai::StimulusType::EventFitAgain)
    );
}

#[test]
fn deferred_wakeup_soldier_skips_blink_when_npcs_cannot_be_enemies() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::combat::ConcussionOutcome;
    use crate::element::Camp;

    let mut engine = EngineInner::new();
    let waker = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let opposite_camp_npc = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    install_seen_enemy(&mut engine, opposite_camp_npc, waker);

    engine
        .orders
        .pending_concussion_side_effects
        .push((waker, ConcussionOutcome::WokeUp));
    engine.drain_pending_concussion_side_effects(sim, &LevelAssets::new());

    engine.apply_wake_redetection_blinks(waker);
    assert_eq!(enemy_blink_state(&engine, opposite_camp_npc), (true, true));
}
