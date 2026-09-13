use super::*;
use crate::element::{
    ActionState, ActorData, ActorPc, ActorSoldier, ElementData, ElementKind, Entity, HumanData,
    NpcData, PcData, Posture, SoldierData,
};
use crate::order::OrderType;
use crate::sequence::{SequenceElement, SequenceId, SequenceState};

fn make_aiming_pc(action_state: ActionState) -> Entity {
    Entity::Pc(ActorPc {
        element: {
            let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
            initial_element.kind = ElementKind::ActorPc;
            initial_element.active = true;
            initial_element
        },
        actor: ActorData {
            action_state,
            ..ActorData::default()
        },
        human: HumanData::default(),
        pc: PcData::default(),
    })
}

fn launch_bow_command_and_tick(command: Command, action_state: ActionState) -> EngineInner {
    let mut engine = EngineInner::new();
    let pc_id = engine.add_test_entity(make_aiming_pc(action_state));
    engine.launch_element(SequenceElement::new(1, command, Some(pc_id)));

    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = engine.test_runtime_assets();
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
    engine
}

fn command_order_types(engine: &EngineInner) -> Vec<OrderType> {
    engine
        .orders
        .sequence_manager
        .get_element(SequenceId(1), 0)
        .unwrap()
        .orders
        .iter()
        .map(|order| order.order_type)
        .collect()
}

fn make_bow_soldier(posture: Posture, action_state: ActionState) -> Entity {
    Entity::Soldier(ActorSoldier {
        element: {
            let mut initial_element = ElementData::from_initial_posture(posture);
            initial_element.kind = ElementKind::ActorSoldier;
            initial_element.active = true;
            initial_element
        },
        actor: ActorData {
            action_state,
            ..ActorData::default()
        },
        human: HumanData::default(),
        npc: NpcData::default(),
        soldier: SoldierData::default(),
    })
}

fn install_test_lift_sector(
    engine: &mut EngineInner,
    owner: EntityId,
    sector_number: crate::sector::SectorNumber,
) {
    engine
        .world
        .entities
        .get_mut(owner)
        .expect("test lift owner exists")
        .element_data_mut()
        .set_sector(crate::position_interface::SectorHandle::new(0));
    let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
    level.sector_number_map.insert(sector_number, 0);
    level.sectors.push(crate::fast_find_grid::GridSector {
        points: Vec::new(),
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type: crate::sector::SectorType::LIFT,
        layer: 0,
        sector_number,
        door_index: None,
        lift_type: Some(crate::sector::LiftType::Ladder),
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    });
}

#[test]
fn bow_lean_out_commands_keep_transition_order_live() {
    let mut engine = EngineInner::new();
    let soldier_id = engine.add_test_entity(make_bow_soldier(
        Posture::Upright,
        ActionState::AimingWithBow,
    ));
    let seq_id = engine.launch_element(SequenceElement::new(
        1,
        Command::LowerBowLeanOut,
        Some(soldier_id),
    ));

    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = engine.test_runtime_assets();
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);

    let elem = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    assert_eq!(
        elem.state,
        SequenceState::InProgress,
        "lower-bow lean-out keeps its translated transition order live"
    );
    assert_eq!(
        elem.current_order().map(|order| order.order_type),
        Some(OrderType::TransitionLoweringBowLeaningOut)
    );
}

#[test]
fn equip_bow_terminates_when_actor_is_already_aiming() {
    let engine = launch_bow_command_and_tick(Command::EquipBow, ActionState::AimingWithBow);
    let elem = engine
        .orders
        .sequence_manager
        .get_element(SequenceId(1), 0)
        .unwrap();

    assert_eq!(elem.state, SequenceState::Terminated);
    assert!(
        elem.orders.is_empty(),
        "redundant EquipBow must not queue equip/load orders"
    );
}

#[test]
fn pre_timer_condolation_starts_successor_timer_before_the_scan() {
    use crate::sequence::{Field, FieldValue, Sequence};

    let mut engine = EngineInner::new();
    let pc_id = engine.add_test_entity(make_aiming_pc(ActionState::AimingWithBow));
    let mut sequence = Sequence::new();
    sequence.append_element(SequenceElement::new(1, Command::EquipBow, Some(pc_id)));
    let mut timer = SequenceElement::new_generic(2, Command::Timer, None);
    timer.set_property(Field::Timer, FieldValue::Integer(2));
    sequence.append_element(timer);
    engine.orders.sequence_manager.launch_sequence(sequence);

    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = engine.test_runtime_assets();
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);

    assert_eq!(engine.orders.timer_elements.len(), 1);
    assert_eq!(
        engine.orders.timer_elements[0].remaining, 1,
        "the immediate Timer successor must launch before the same frame's timer scan"
    );
}

#[test]
fn timer_expiry_condolation_starts_successor_after_the_scan() {
    use crate::sequence::{Field, FieldValue, Sequence};

    let mut engine = EngineInner::new();
    let pc_id = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
    let mut sequence = Sequence::new();
    let mut expiring = SequenceElement::new_generic(1, Command::Timer, Some(pc_id));
    expiring.set_property(Field::Timer, FieldValue::Integer(1));
    sequence.append_element(expiring);
    let mut successor = SequenceElement::new_generic(2, Command::Timer, None);
    successor.set_property(Field::Timer, FieldValue::Integer(2));
    sequence.append_element(successor);
    engine.orders.sequence_manager.launch_sequence(sequence);

    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = engine.test_runtime_assets();
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);

    assert_eq!(engine.orders.timer_elements.len(), 1);
    assert_eq!(
        engine.orders.timer_elements[0].remaining, 2,
        "a successor launched by timer expiry belongs to the final condolation drain and must not re-enter the timer scan in progress"
    );
}

#[test]
fn equip_bow_down_terminates_when_actor_is_already_aiming_up() {
    let engine = launch_bow_command_and_tick(Command::EquipBowDown, ActionState::AimingWithBowUp);
    let elem = engine
        .orders
        .sequence_manager
        .get_element(SequenceId(1), 0)
        .unwrap();

    assert_eq!(elem.state, SequenceState::Terminated);
    assert!(
        elem.orders.is_empty(),
        "redundant EquipBowDown must not queue equip/load/lower orders"
    );
}

#[test]
fn raise_bow_from_waiting_queues_equip_load_then_raise() {
    let engine = launch_bow_command_and_tick(Command::RaiseBow, ActionState::Waiting);

    assert_eq!(
        command_order_types(&engine),
        vec![
            OrderType::TransitionEquipBow,
            OrderType::TransitionLoadingBow,
            OrderType::TransitionRaisingBow,
        ],
        "bow aiming raises from waiting by equipping, loading, then raising"
    );
}

#[test]
fn unequip_bow_from_aiming_up_queues_lower_unload_then_unequip() {
    let engine = launch_bow_command_and_tick(Command::UnequipBow, ActionState::AimingWithBowUp);

    assert_eq!(
        command_order_types(&engine),
        vec![
            OrderType::TransitionLoweringBow,
            OrderType::TransitionUnloadBow,
            OrderType::TransitionUnequipBow,
        ],
        "bow aiming exits bow-up by lowering, unloading, then unequipping"
    );
}

#[test]
fn turn_context_sets_goal_without_snapping_and_books_turning() {
    use crate::sequence::{Field, FieldValue};

    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
    let mut turn = SequenceElement::new_generic(1, Command::Turn, Some(owner));
    turn.set_property(Field::Direction, FieldValue::Integer(5));
    let seq_id = engine.orders.sequence_manager.launch_element(turn);

    let barrier = TurnCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        orders: crate::engine::sequence_runtime::OrderEmitter::new(
            &mut engine.orders.next_order_id,
        ),
    }
    .dispatch(owner, Command::Turn, seq_id, 0);

    assert_eq!(barrier, OwnerActionBarrier::Reach);
    let entity = engine.world.entities.get(owner).unwrap();
    assert_eq!(entity.element_data().direction(), 0);
    assert_eq!(
        u8::from(
            entity
                .element_data()
                .sprite
                .position_iface
                .get_direction_goal()
        ),
        5,
        "Turn must set the progressive direction goal, not snap direction"
    );
    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    assert_eq!(element.state, SequenceState::InProgress);
    assert_eq!(
        element.current_order().map(|order| order.order_type),
        Some(OrderType::Turning)
    );
    assert!(
        !element.current_order().unwrap().compute_direction,
        "Turn translation already resolved the direction goal and must not recompute it from the order's dummy point"
    );
}

#[test]
fn wait_timer_context_arms_actor_and_books_upright_idle() {
    use crate::sequence::{Field, FieldValue};

    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
    let mut wait = SequenceElement::new_generic(1, Command::WaitTimer, Some(owner));
    wait.set_property(Field::Timer, FieldValue::Integer(7));
    let seq_id = engine.orders.sequence_manager.launch_element(wait);

    let barrier = WaitCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        orders: crate::engine::sequence_runtime::OrderEmitter::new(
            &mut engine.orders.next_order_id,
        ),
        profiles: &assets.profile_manager,
    }
    .dispatch(owner, Command::WaitTimer, seq_id, 0);

    assert_eq!(barrier, OwnerActionBarrier::Reach);
    assert_eq!(
        engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .wait_time,
        7
    );
    assert_eq!(
        engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .seek_refresh_wait,
        7,
        "WAIT_TIMER writes Original's shared mulWaitTime, so every Rust storage mirror must retain the same value across interruption"
    );
    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    assert_eq!(element.state, SequenceState::InProgress);
    assert_eq!(
        element.current_order().map(|order| order.order_type),
        Some(OrderType::WaitingUprightBored)
    );
    assert!(!element.current_order().unwrap().compute_direction);

    // A timer may interrupt a seek while the actor-owned post-seek
    // pointers remain dormant. Once the timer itself is interrupted and
    // the actor falls back to Wait, the parity view must still expose the
    // last value written to the original game's one shared wait-timer scalar.
    {
        let actor = engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.seek_target = Some(owner);
        actor.post_seek_sequence = Some(crate::sequence::Sequence::new().into_post_seek());
    }
    engine.orders.sequence_manager.element_interrupted(
        seq_id,
        0,
        crate::sequence::CascadeFlags::NEXT_LEVEL,
    );
    let mut idle = SequenceElement::new(1, Command::Wait, Some(owner));
    idle.priority = crate::sequence::SequencePriority::Wait;
    let idle_sequence = engine.orders.sequence_manager.launch_element(idle);
    engine
        .orders
        .sequence_manager
        .element_in_progress(idle_sequence, 0);
    assert_eq!(engine.actor_legacy_wait_time(owner), 7);

    // Savegame_linux3/Profile_003/Savegame_065 replay-003 frame
    // 16245: a long jump starts while these post-seek pointers remain
    // retained. The original game's airborne execution branch overwrites the wait timer
    // with the flight duration, so that live owner must take precedence
    // over the dormant seek-refresh copy.
    {
        use crate::engine::jump::{ActiveJump, CurrentStepState, JumpStep};
        use crate::sequence::SequenceId;
        use std::collections::VecDeque;
        use std::num::NonZeroU32;

        let actor = engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.wait_time = 4;
        actor.seek_refresh_wait = 0;
        actor.active_jump = Some(ActiveJump {
            steps: VecDeque::new(),
            current: Some(CurrentStepState {
                start_x: 0.0,
                start_y: 0.0,
                start_z: 0.0,
                total_frames: 5,
                frames_elapsed: 1,
                order_id: NonZeroU32::new(1).unwrap(),
                airborne_increment: None,
                step: JumpStep {
                    anim: OrderType::JumpingLong,
                    target_3d: None,
                    airborne: true,
                    max_frames: None,
                },
            }),
            sequence_id: SequenceId(1),
            element_index: 0,
            dest_sector: None,
            dest_layer: 0,
            source_direction_goal: 0,
            dest_projection_point: crate::coordinates::MapPoint::default(),
        });
    }
    assert_eq!(engine.actor_legacy_wait_time(owner), 4);
    engine
        .world
        .entities
        .get_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_jump
        .as_mut()
        .unwrap()
        .current
        .as_mut()
        .unwrap()
        .step
        .airborne = false;
    assert_eq!(engine.actor_legacy_wait_time(owner), 0);
}

#[test]
fn ladder_fall_wait_owns_legacy_scalar_over_dormant_seek() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_aiming_pc(ActionState::Moving));
    let actor = engine
        .world
        .entities
        .get_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap();

    // A swordstrike post-seek may remain attached while a non-interruptible
    // ladder/wall fall runs. The original game's ladder/wall fall execution owns
    // the single wait-timer scalar for the flight countdown in this state.
    actor.seek_target = Some(owner);
    actor.post_seek_sequence = Some(crate::sequence::Sequence::new().into_post_seek());
    actor.seek_refresh_wait = 0;
    actor.wait_time = 2;
    actor.active_flight = Some(crate::element::ActiveFlight {
        frames_remaining: 2,
        ladder_fall: true,
        ..crate::element::ActiveFlight::default()
    });

    assert_eq!(engine.actor_legacy_wait_time(owner), 2);
}

#[test]
fn frozen_all_wait_timer_still_completes_in_owner_slot() {
    use crate::sequence::{Field, FieldValue};

    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
    let mut wait = SequenceElement::new_generic(1, Command::WaitTimer, Some(owner));
    wait.set_property(Field::Timer, FieldValue::Integer(0));
    let seq_id = engine.orders.sequence_manager.launch_element(wait);
    WaitCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        orders: crate::engine::sequence_runtime::OrderEmitter::new(
            &mut engine.orders.next_order_id,
        ),
        profiles: &assets.profile_manager,
    }
    .dispatch(owner, Command::WaitTimer, seq_id, 0);
    let _ = engine
        .orders
        .sequence_manager
        .take_pending_synchronous_actions();
    engine.set_actors_frozen(true);

    engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .expect("wait timer remains inspectable")
            .state,
        SequenceState::Terminated
    );
}

#[test]
fn wait_timer_wraps_beggar_execute_and_generic_execute_once_each() {
    fn run_once(order_type: OrderType, wait_time: u32) -> (u32, SequenceState) {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let mut owner_entity = make_aiming_pc(ActionState::Waiting);
        let mut conversion = crate::engine::test_support::unmapped_conversion();
        conversion[order_type as usize] = 0;
        owner_entity.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                action_id: order_type as u16,
                action_done: 0,
                frame_ids: vec![1],
                delays: vec![10],
                distances: vec![0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
                sound_ids: vec![0],
                ..Default::default()
            }]),
            std::sync::Arc::new(conversion),
        );
        owner_entity.actor_data_mut().unwrap().wait_time = wait_time;
        owner_entity.actor_data_mut().unwrap().seek_refresh_wait = wait_time;
        let owner = engine.add_test_entity(owner_entity);

        let mut wait = SequenceElement::new_generic(1, Command::WaitTimer, Some(owner));
        wait.priority = crate::sequence::SequencePriority::Normal;
        let seq_id = engine.orders.sequence_manager.launch_element(wait);
        engine
            .orders
            .sequence_manager
            .element_in_progress(seq_id, 0);
        let order_id = engine.orders.allocate_order_id();
        engine.orders.sequence_manager.push_order_on(
            seq_id,
            0,
            crate::order::Order::new(order_type, 0.0, 0.0, order_id),
        );

        engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);
        let remaining = engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .wait_time;
        let state = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .expect("WAIT_TIMER remains inspectable")
            .state;
        (remaining, state)
    }

    // Savegame_linux2/Profile_002/Savegame_032 replay-008 frame
    // 17705 enters with WAIT_TIMER=23 and SIMULATING_BEGGAR selected.
    // The original game's base actor update decrements it after the player
    // override returns; the specialized Rust arm used to skip that base
    // modifier and retain 23.
    assert_eq!(run_once(OrderType::SimulatingBeggar, 23).0, 22);
    assert_eq!(
        run_once(OrderType::WaitingUpright, 23).0,
        22,
        "the generic Execute path must retain its single decrement"
    );
    assert_eq!(
        run_once(OrderType::SimulatingBeggar, 0).1,
        SequenceState::Terminated,
        "the specialized Execute result must carry WAIT_TIMER termination into base completion"
    );
}

#[test]
fn lazy_wait_publishes_start_before_preexisting_owner_instruction() {
    use crate::sequence::SequenceAction;

    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let mut owner_entity = make_aiming_pc(ActionState::Moving);
    let mut conversion = crate::engine::test_support::unmapped_conversion();
    conversion[OrderType::TransitionWalkingUprightWaitingUpright as usize] = 0;
    conversion[OrderType::WalkingUpright as usize] = 1;
    conversion[OrderType::WaitingUpright as usize] = 2;
    let script = |action: OrderType| crate::sprite_script::SpriteScript {
        action_id: action as u16,
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
    owner_entity.element_data_mut().sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![
            script(OrderType::TransitionWalkingUprightWaitingUpright),
            script(OrderType::WalkingUpright),
            script(OrderType::WaitingUpright),
        ]),
        std::sync::Arc::new(conversion),
    );
    owner_entity.element_data_mut().sprite.current_row = 1;
    owner_entity.element_data_mut().sprite.last_action = OrderType::WalkingUpright;
    let owner = engine.add_test_entity(owner_entity);
    let parry_sequence =
        engine.launch_element(SequenceElement::new(1, Command::ParrySword, Some(owner)));
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(parry_sequence, 0)
            .expect("ParrySword remains queued for manager dispatch")
            .priority,
        crate::sequence::SequencePriority::NotYetSet,
        "sequence-element launch must leave ordinary work unresolved until manager instruction"
    );

    engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

    let sprite = &engine
        .world
        .entities
        .get(owner)
        .unwrap()
        .element_data()
        .sprite;
    assert_eq!(
        sprite.last_action,
        OrderType::TransitionWalkingUprightWaitingUpright,
        "synchronous synthetic Wait must publish its transition START before later owner work"
    );
    assert_eq!(sprite.current_row, 0);
    assert_eq!(sprite.current_frame, 0);
    assert_eq!(sprite.frame_count, u16::MAX);
    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .is_some_and(|(_, _, order)| {
                order.order_type == OrderType::TransitionWalkingUprightWaitingUpright
            }),
        "the transient Wait remains selected until deferred owner work is processed"
    );
    let pending = engine.orders.sequence_manager.hourglass();
    assert_eq!(pending.len(), 1);
    let pending_ids = pending
        .iter()
        .map(|action| match action {
            SequenceAction::InstructOwner {
                owner: action_owner,
                sequence_id,
                element_index: 0,
            } if *action_owner == owner => *sequence_id,
            other => panic!("unexpected pending action after actor slot: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(pending_ids[0], parry_sequence);
}

#[test]
fn owner_local_stop_movement_new_id_preserves_execute_start() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_aiming_pc(ActionState::Moving));
    let mut movement =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::WalkingUpright);
    movement.priority = crate::sequence::SequencePriority::Normal;
    let sequence_id = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);
    let entry_order_id = engine.orders.allocate_order_id();
    engine.orders.sequence_manager.push_order_on(
        sequence_id,
        0,
        crate::order::Order::new(OrderType::WalkingUpright, 20.0, 0.0, entry_order_id),
    );

    engine.tick_actor_animation_action_change_slots_with_hooks(
        &crate::sim_rng::test_context(),
        &assets,
        |_, _| {},
        |_, _| {},
        |engine, execute_owner, selected_movement, _, _, _, _| {
            assert_eq!(execute_owner, owner);
            assert!(selected_movement.is_some());
            engine
                .world
                .entities
                .get_mut(owner)
                .unwrap()
                .element_data_mut()
                .sprite
                .last_motion_state = Some(crate::sprite::MotionState::Start);
            // A LINE_SCRIPT EnterZone callback can invoke StopActor here,
            // after execution has produced START but before the actor update
            // performs its completion projection.
            engine.stop_owner(owner, crate::sequence::SequencePriority::Script);
        },
        |_, _, _| {},
    );

    let actor = engine
        .world
        .entities
        .get(owner)
        .unwrap()
        .actor_data()
        .unwrap();
    assert_eq!(
        actor.continuation.motion_state,
        crate::sprite::MotionState::Start
    );
    let (_, _, rewritten) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(owner)
        .expect("stopped walking order remains selected as its transition");
    assert_eq!(
        rewritten.order_type,
        OrderType::TransitionWalkingUprightWaitingUpright
    );
    assert_ne!(rewritten.order_id, entry_order_id);
}

#[test]
fn fresh_waypoint_start_advancing_to_older_stop_transition_is_in_progress() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_aiming_pc(ActionState::MovingFast));
    let mut movement =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::RunningUpright);
    movement.priority = crate::sequence::SequencePriority::Normal;
    let sequence_id = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);
    // Path postprocessing allocates its final transition before inserting
    // path waypoints ahead of it, so the waypoint has the newer ID.
    let transition_order_id = engine.orders.allocate_order_id();
    let waypoint_order_id = engine.orders.allocate_order_id();
    engine.orders.sequence_manager.push_order_on(
        sequence_id,
        0,
        crate::order::Order::new(OrderType::RunningUpright, 20.0, 0.0, waypoint_order_id),
    );
    engine.orders.sequence_manager.push_order_on(
        sequence_id,
        0,
        crate::order::Order::new(
            OrderType::TransitionRunningUprightWaitingUpright,
            20.0,
            0.0,
            transition_order_id,
        ),
    );

    engine.tick_actor_animation_action_change_slots_with_hooks(
        &crate::sim_rng::test_context(),
        &assets,
        |_, _| {},
        |_, _| {},
        |engine, execute_owner, selected_movement, _, _, _, _| {
            assert_eq!(execute_owner, owner);
            assert!(selected_movement.is_some());
            engine
                .world
                .entities
                .get_mut(owner)
                .unwrap()
                .element_data_mut()
                .sprite
                .last_motion_state = Some(crate::sprite::MotionState::Start);
            engine.do_next_order(sequence_id, 0);
        },
        |_, _, _| {},
    );

    let actor = engine
        .world
        .entities
        .get(owner)
        .unwrap()
        .actor_data()
        .unwrap();
    assert_eq!(
        actor.continuation.motion_state,
        crate::sprite::MotionState::InProgress
    );
    let (_, _, successor) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(owner)
        .expect("pre-existing stop transition must remain selected");
    assert_eq!(successor.order_id, transition_order_id);
    assert_eq!(
        successor.order_type,
        OrderType::TransitionRunningUprightWaitingUpright
    );
}

#[test]
fn npc_state_context_preserves_menace_order_and_reaches_splice_barrier() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
    let seq_id = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::StartMenace, Some(owner)));

    let barrier = NpcStateCommandContext {
        sequence_manager: &mut engine.orders.sequence_manager,
        orders: crate::engine::sequence_runtime::OrderEmitter::new(
            &mut engine.orders.next_order_id,
        ),
    }
    .dispatch(Command::StartMenace, seq_id, 0);

    assert_eq!(barrier, OwnerActionBarrier::Reach);
    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    assert_eq!(element.state, SequenceState::InProgress);
    assert_eq!(
        element
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![
            OrderType::TransitionRaisingSword,
            OrderType::TransitionWaitingSwordMenacing,
        ]
    );
    assert!(element.orders.iter().all(|order| !order.compute_direction));
}

#[test]
fn npc_attention_context_uses_alerted_look_and_reaches_splice_barrier() {
    let mut engine = EngineInner::new();
    let mut soldier_entity = make_bow_soldier(Posture::Upright, ActionState::Waiting);
    let Entity::Soldier(soldier) = &mut soldier_entity else {
        unreachable!();
    };
    soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    let owner = engine.add_test_entity(soldier_entity);
    engine
        .world
        .entities
        .get_mut(owner)
        .and_then(Entity::enemy_ai_mut)
        .expect("test soldier has enemy AI")
        .attentive = true;
    let seq_id = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::LookLeft, Some(owner)));

    let barrier = NpcAttentionCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        orders: crate::engine::sequence_runtime::OrderEmitter::new(
            &mut engine.orders.next_order_id,
        ),
    }
    .dispatch(owner, Command::LookLeft, seq_id, 0);

    assert_eq!(barrier, OwnerActionBarrier::Reach);
    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    assert_eq!(element.state, SequenceState::InProgress);
    assert_eq!(
        element.current_order().map(|order| order.order_type),
        Some(OrderType::LookingLeftAlerted)
    );
    assert!(!element.current_order().unwrap().compute_direction);
}

#[test]
fn stealth_context_crouches_and_preserves_terminated_order() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
    let seq_id = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::CrouchDown, Some(owner)));

    let barrier = StealthCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        orders: crate::engine::sequence_runtime::OrderEmitter::new(
            &mut engine.orders.next_order_id,
        ),
        titbit_manager: &mut engine.feedback.titbit_manager,
        profiles: &assets.profile_manager,
    }
    .dispatch(owner, Command::CrouchDown, seq_id, 0);

    assert_eq!(barrier, OwnerActionBarrier::Reach);
    // Translation only appends the crouch transition order; the posture
    // and action-state snap happen when the transition animation reaches
    // DONE, so the element stays selected/in-progress and the actor is
    // untouched during dispatch.
    let entity = engine.world.entities.get(owner).unwrap();
    assert_eq!(entity.element_data().posture(), Posture::Upright);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::Waiting
    );
    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    assert_eq!(element.state, SequenceState::InProgress);
    assert_eq!(
        element.current_order().map(|order| order.order_type),
        Some(OrderType::TransitionCrouchingDown),
        "the crouch body only queues its transition order at translation time"
    );
}

#[test]
#[should_panic(expected = "WAIT_TIMER owner")]
fn wait_timer_context_rejects_missing_timer_contextually() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
    let wait = SequenceElement::new_generic(1, Command::WaitTimer, Some(owner));
    let seq_id = engine.orders.sequence_manager.launch_element(wait);

    WaitCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        orders: crate::engine::sequence_runtime::OrderEmitter::new(
            &mut engine.orders.next_order_id,
        ),
        profiles: &assets.profile_manager,
    }
    .dispatch(owner, Command::WaitTimer, seq_id, 0);
}

#[test]
#[should_panic(expected = "Wait translation owner")]
fn wait_context_rejects_stale_owner_contextually() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
    let wait = SequenceElement::new(1, Command::Wait, Some(owner));
    let seq_id = engine.orders.sequence_manager.launch_element(wait);
    engine.remove_entity(owner);

    WaitCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        orders: crate::engine::sequence_runtime::OrderEmitter::new(
            &mut engine.orders.next_order_id,
        ),
        profiles: &assets.profile_manager,
    }
    .dispatch(owner, Command::Wait, seq_id, 0);
}

#[test]
fn stealth_termination_splices_timer_successor_before_same_tick_scan() {
    use crate::sequence::{Field, FieldValue, Sequence};

    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
    // Crouch bodies now stay live until their transition animation
    // completes, so use LeaveSpy — the stealth context still snaps the
    // posture and terminates it synchronously inside its dispatch slot.
    engine
        .world
        .entities
        .get_mut(owner)
        .unwrap()
        .element_data_mut()
        .publish_order_posture(Posture::Spy);
    let mut sequence = Sequence::new();
    // Production launches LeaveSpy with the auto-leave helper, which
    // never reaches priority arbitration; preset the priority the way
    // an already-arbitrated element carries it.
    let mut leave = SequenceElement::new(1, Command::LeaveSpy, Some(owner));
    leave.priority = crate::sequence::SequencePriority::Normal;
    sequence.append_element(leave);
    let mut timer = SequenceElement::new_generic(2, Command::Timer, None);
    timer.set_property(Field::Timer, FieldValue::Integer(2));
    sequence.append_element(timer);
    engine.orders.sequence_manager.launch_sequence(sequence);

    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = engine.test_runtime_assets();
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);

    assert_eq!(engine.orders.timer_elements.len(), 1);
    assert_eq!(
        engine.orders.timer_elements[0].remaining, 1,
        "the stealth context must reach the synchronous splice before the timer scan"
    );
}

#[test]
fn direct_ability_context_starts_whistle_and_reaches_splice_barrier() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
    let seq_id = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::WhistleCmd, Some(owner)));

    let barrier = DirectAbilityCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        orders: crate::engine::sequence_runtime::OrderEmitter::new(
            &mut engine.orders.next_order_id,
        ),
        profiles: &assets.profile_manager,
    }
    .dispatch(owner, Command::WhistleCmd, true, seq_id, 0);

    assert_eq!(barrier, OwnerActionBarrier::Reach);
    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    assert_eq!(element.state, SequenceState::InProgress);
    assert_eq!(
        element.current_order().map(|order| order.order_type),
        Some(OrderType::Whistling)
    );
    assert_eq!(
        engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .whistle_wait_time,
        25
    );
}

#[test]
fn direct_ability_context_preserves_eat_no_ammo_skip_barrier() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
    let seq_id = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::EatCmd, Some(owner)));

    let barrier = DirectAbilityCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        orders: crate::engine::sequence_runtime::OrderEmitter::new(
            &mut engine.orders.next_order_id,
        ),
        profiles: &assets.profile_manager,
    }
    .dispatch(owner, Command::EatCmd, false, seq_id, 0);

    assert_eq!(barrier, OwnerActionBarrier::Skip);
    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    assert_eq!(element.state, SequenceState::Terminated);
    assert!(element.orders.is_empty());
}

#[test]
fn direct_ability_context_preserves_missing_throw_target_skip_barrier() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_aiming_pc(ActionState::Waiting));
    let seq_id = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::ThrowApple, Some(owner)));

    let barrier = DirectAbilityCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        orders: crate::engine::sequence_runtime::OrderEmitter::new(
            &mut engine.orders.next_order_id,
        ),
        profiles: &assets.profile_manager,
    }
    .dispatch(owner, Command::ThrowApple, true, seq_id, 0);

    assert_eq!(barrier, OwnerActionBarrier::Skip);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap()
            .state,
        SequenceState::Impossible
    );
}

#[test]
fn position_assertion_context_interrupts_at_tolerance_boundary() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
    let mut assertion = SequenceElement::new_movement(
        1,
        Command::AssertPosition,
        Some(owner),
        OrderType::WalkingUpright,
    );
    if let crate::sequence::SequenceElementData::Movement {
        destination,
        tolerance,
        ..
    } = &mut assertion.data
    {
        *destination = crate::coordinates::MapPoint::new(5.0, 0.0);
        *tolerance = 0.0;
    }
    let seq_id = engine.orders.sequence_manager.launch_element(assertion);

    let barrier = PositionAssertionContext {
        entities: &engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
    }
    .dispatch(owner, seq_id, 0);

    assert_eq!(barrier, OwnerActionBarrier::Skip);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap()
            .state,
        SequenceState::Interrupted,
        "movement uses >= tolerance + 5 for the max-norm mismatch"
    );
}

#[test]
fn position_assertion_context_accepts_nan_distance_like_original() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
    engine
        .world
        .entities
        .get_mut(owner)
        .expect("test assertion owner")
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::new(f32::NAN, f32::NAN));
    let mut assertion = SequenceElement::new_movement(
        1,
        Command::AssertPosition,
        Some(owner),
        OrderType::WalkingUpright,
    );
    if let crate::sequence::SequenceElementData::Movement {
        destination,
        tolerance,
        ..
    } = &mut assertion.data
    {
        *destination = crate::coordinates::MapPoint::new(362.0, 1535.0);
        *tolerance = 10.0;
    }
    let seq_id = engine.orders.sequence_manager.launch_element(assertion);

    let barrier = PositionAssertionContext {
        entities: &engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
    }
    .dispatch(owner, seq_id, 0);

    assert_eq!(barrier, OwnerActionBarrier::Skip);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap()
            .state,
        SequenceState::Terminated,
        "Original's `qNaN >= tolerance + 5` mismatch test is false"
    );
}

#[test]
fn lift_wait_context_keeps_blocked_lift_in_progress_and_reaches_splice() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
    let sector_number = crate::sector::SectorNumber::new(42);
    install_test_lift_sector(&mut engine, owner, sector_number);
    engine.world.fast_grid_mut().lift_state_mut(0).wait_time = 2;
    let door = crate::gate::Door {
        door_type: crate::gate::DoorType::LiftHigh,
        sector_in: sector_number,
        ..crate::gate::Door::default()
    };
    let mut wait = SequenceElement::new_movement(
        1,
        Command::WaitFreeLift,
        Some(owner),
        OrderType::WalkingUpright,
    );
    if let crate::sequence::SequenceElementData::Movement {
        gate_id, sector, ..
    } = &mut wait.data
    {
        *gate_id = Some(crate::gate::DoorIndex::new(0).expect("valid door index"));
        *sector = crate::position_interface::SectorHandle::new(42);
    }
    let seq_id = engine.orders.sequence_manager.launch_element(wait);

    WaitCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        orders: crate::engine::sequence_runtime::OrderEmitter::new(
            &mut engine.orders.next_order_id,
        ),
        profiles: &assets.profile_manager,
    }
    .dispatch(owner, Command::WaitFreeLift, seq_id, 0);

    let authorized = LiftWaitCommandContext {
        entities: &mut engine.world.entities,
        fast_grid: std::sync::Arc::make_mut(&mut engine.world.fast_grid),
        doors: std::slice::from_ref(&door),
        sequence_manager: &mut engine.orders.sequence_manager,
    }
    .authorize_and_reserve(owner, seq_id, 0);

    assert!(!authorized);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );
    assert_eq!(engine.world.fast_grid_mut().lift_state_mut(0).wait_time, 1);
    assert!(
        engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_lift
            .is_none()
    );
}

#[test]
#[should_panic(expected = "must be LiftHigh or LiftLow")]
fn lift_wait_context_rejects_crenel_lift_type_contextually() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
    let sector_number = crate::sector::SectorNumber::new(42);
    install_test_lift_sector(&mut engine, owner, sector_number);
    let door = crate::gate::Door {
        door_type: crate::gate::DoorType::LiftHighCrenel,
        sector_in: sector_number,
        ..crate::gate::Door::default()
    };
    let mut wait = SequenceElement::new_movement(
        1,
        Command::WaitFreeLift,
        Some(owner),
        OrderType::WalkingUpright,
    );
    if let crate::sequence::SequenceElementData::Movement {
        gate_id, sector, ..
    } = &mut wait.data
    {
        *gate_id = Some(crate::gate::DoorIndex::new(0).expect("valid door index"));
        *sector = crate::position_interface::SectorHandle::new(42);
    }
    let seq_id = engine.orders.sequence_manager.launch_element(wait);
    WaitCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        orders: crate::engine::sequence_runtime::OrderEmitter::new(
            &mut engine.orders.next_order_id,
        ),
        profiles: &assets.profile_manager,
    }
    .dispatch(owner, Command::WaitFreeLift, seq_id, 0);

    LiftWaitCommandContext {
        entities: &mut engine.world.entities,
        fast_grid: std::sync::Arc::make_mut(&mut engine.world.fast_grid),
        doors: std::slice::from_ref(&door),
        sequence_manager: &mut engine.orders.sequence_manager,
    }
    .authorize_and_reserve(owner, seq_id, 0);
}

#[test]
fn lift_wait_context_reserves_direction_before_terminating() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
    let sector_number = crate::sector::SectorNumber::new(42);
    install_test_lift_sector(&mut engine, owner, sector_number);
    let door = crate::gate::Door {
        door_type: crate::gate::DoorType::LiftHigh,
        sector_in: sector_number,
        ..crate::gate::Door::default()
    };
    let mut wait = SequenceElement::new_movement(
        1,
        Command::WaitFreeLift,
        Some(owner),
        OrderType::WalkingUpright,
    );
    if let crate::sequence::SequenceElementData::Movement {
        gate_id, sector, ..
    } = &mut wait.data
    {
        *gate_id = Some(crate::gate::DoorIndex::new(0).expect("valid door index"));
        *sector = crate::position_interface::SectorHandle::new(42);
    }
    let seq_id = engine.orders.sequence_manager.launch_element(wait);

    WaitCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        orders: crate::engine::sequence_runtime::OrderEmitter::new(
            &mut engine.orders.next_order_id,
        ),
        profiles: &assets.profile_manager,
    }
    .dispatch(owner, Command::WaitFreeLift, seq_id, 0);

    let authorized = LiftWaitCommandContext {
        entities: &mut engine.world.entities,
        fast_grid: std::sync::Arc::make_mut(&mut engine.world.fast_grid),
        doors: std::slice::from_ref(&door),
        sequence_manager: &mut engine.orders.sequence_manager,
    }
    .authorize_and_reserve(owner, seq_id, 0);

    assert!(authorized);
    engine.do_next_order(seq_id, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    let lift = engine.world.fast_grid_mut().lift_state_mut(0);
    assert_eq!(lift.occupants, 1);
    assert!(lift.occupied_downwards);
    assert_eq!(lift.wait_time, 100);
    let active_lift = engine
        .world
        .entities
        .get(owner)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_lift
        .expect("authorized actor records its active lift");
    assert_eq!(active_lift.sector_number, 42);
    assert!(!active_lift.upwards);
}

#[test]
fn lift_wait_reservation_is_consumed_by_production_leave_callback() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
    let sector_number = crate::sector::SectorNumber::new(42);
    install_test_lift_sector(&mut engine, owner, sector_number);
    {
        let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
        let outside = crate::sector::SectorNumber::new(0);
        let outside_index = level.sectors.len();
        level.sector_number_map.insert(outside, outside_index);
        level.sectors.push(crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
            layer: 0,
            sector_number: outside,
            door_index: None,
            lift_type: None,
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        });
    }
    let door = crate::gate::Door {
        door_type: crate::gate::DoorType::LiftHigh,
        sector_in: sector_number,
        sector_out: crate::sector::SectorNumber::new(0),
        sector_in_index: crate::fast_find_grid::SectorIndex::new(0),
        sector_out_index: crate::fast_find_grid::SectorIndex::new(1),
        ..crate::gate::Door::default()
    };
    engine.script_domains.interactables.doors.push(door.clone());
    let mut wait = SequenceElement::new_movement(
        1,
        Command::WaitFreeLift,
        Some(owner),
        OrderType::WalkingUpright,
    );
    if let crate::sequence::SequenceElementData::Movement {
        gate_id, sector, ..
    } = &mut wait.data
    {
        *gate_id = Some(crate::gate::DoorIndex::new(0).expect("valid door index"));
        *sector = crate::position_interface::SectorHandle::new(42);
    }
    let seq_id = engine.orders.sequence_manager.launch_element(wait);
    WaitCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        orders: crate::engine::sequence_runtime::OrderEmitter::new(
            &mut engine.orders.next_order_id,
        ),
        profiles: &assets.profile_manager,
    }
    .dispatch(owner, Command::WaitFreeLift, seq_id, 0);

    assert!(
        LiftWaitCommandContext {
            entities: &mut engine.world.entities,
            fast_grid: std::sync::Arc::make_mut(&mut engine.world.fast_grid),
            doors: std::slice::from_ref(&door),
            sequence_manager: &mut engine.orders.sequence_manager,
        }
        .authorize_and_reserve(owner, seq_id, 0)
    );
    assert_eq!(engine.world.fast_grid_mut().lift_state_mut(0).occupants, 1);

    engine.execute_pass_door(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        crate::gate::DoorIndex::new(0).expect("valid door index"),
        true,
        0,
    );
    assert_eq!(engine.world.fast_grid_mut().lift_state_mut(0).occupants, 1);
    engine.execute_pass_door(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        crate::gate::DoorIndex::new(0).expect("valid door index"),
        false,
        0,
    );

    let lift = engine.world.fast_grid_mut().lift_state_mut(0);
    assert_eq!(lift.occupants, 0);
    assert!(!lift.occupied_downwards);
    assert!(!lift.occupied_upwards);
    assert_eq!(lift.wait_time, 0);
    assert!(
        engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_lift
            .is_none()
    );
}

#[test]
fn frozen_all_lift_wait_rechecks_and_promotes_successor_in_authorizing_slot() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
    let sector_number = crate::sector::SectorNumber::new(42);
    install_test_lift_sector(&mut engine, owner, sector_number);
    crate::engine::test_support::ensure_ordinary_sector(&mut engine, 0, 0);
    engine.world.fast_grid_mut().lift_state_mut(0).wait_time = 2;
    engine
        .script_domains
        .interactables
        .doors
        .push(crate::gate::Door {
            door_type: crate::gate::DoorType::LiftHigh,
            sector_in: sector_number,
            ..crate::gate::Door::default()
        });
    let mut wait = SequenceElement::new_movement(
        1,
        Command::WaitFreeLift,
        Some(owner),
        OrderType::WalkingUpright,
    );
    if let crate::sequence::SequenceElementData::Movement {
        gate_id, sector, ..
    } = &mut wait.data
    {
        *gate_id = Some(crate::gate::DoorIndex::new(0).expect("valid door index"));
        *sector = crate::position_interface::SectorHandle::new(42);
    }
    let seq_id = engine.orders.sequence_manager.launch_element(wait);
    WaitCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        orders: crate::engine::sequence_runtime::OrderEmitter::new(
            &mut engine.orders.next_order_id,
        ),
        profiles: &assets.profile_manager,
    }
    .dispatch(owner, Command::WaitFreeLift, seq_id, 0);
    engine.set_actors_frozen(true);
    let _ = engine
        .orders
        .sequence_manager
        .take_pending_synchronous_actions();
    let sim = crate::sim_rng::test_context();

    engine.tick_actor_animation_action_change_slots(&sim, &assets);
    assert_eq!(engine.world.fast_grid_mut().lift_state_mut(0).wait_time, 1);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .expect("blocked lift wait remains installed")
            .state,
        SequenceState::InProgress
    );

    engine.tick_actor_animation_action_change_slots(&sim, &assets);
    assert_eq!(engine.world.fast_grid_mut().lift_state_mut(0).wait_time, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .expect("zeroed lift wait remains installed")
            .state,
        SequenceState::InProgress,
        "authorization returns false on the frame that decrements the cooldown to zero"
    );

    engine.tick_actor_animation_action_change_slots(&sim, &assets);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .expect("authorized lift wait remains inspectable")
            .state,
        SequenceState::Terminated
    );
    let lift = engine.world.fast_grid_mut().lift_state_mut(0);
    assert_eq!(lift.occupants, 1);
    assert!(lift.occupied_downwards);
    // The fallback idle Wait is no longer installed inside the
    // terminating owner slot: the null-order guard books it at the start
    // of the owner's next actor frame.
    engine.tick_actor_animation_action_change_slots(&sim, &assets);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
            .and_then(|(sequence, element)| engine
                .orders
                .sequence_manager
                .get_element(sequence, element))
            .map(|element| element.command),
        Some(Command::Wait),
        "next-action/wait translation must finish by the owner's next actor frame"
    );
}
