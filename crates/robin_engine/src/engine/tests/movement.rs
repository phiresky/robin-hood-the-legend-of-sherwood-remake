use super::*;

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
    assert_eq!(entity.element_data().position_map(), start);
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

    let mut engine = EngineInner::new();
    let position = MapPoint::new(1352.0, 246.0);
    let mut mover = make_test_pc(Posture::Upright);
    mover.element_data_mut().active = true;
    mover.element_data_mut().set_position_map(position);
    mover.actor_data_mut().unwrap().action_state = ActionState::Moving;
    let mover_id = engine.add_entity(mover);

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
        target_position_observed_by_follow: MapPoint,
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
            positions_before_movement[entity_id] = Some(entity.element_data().position_map());
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
                target_position_observed_by_follow: MapPoint::new(80.0, 20.0),
            },
            Observation {
                frame: 73,
                observer_slot: 1,
                target_slot: 0,
                target_before_movement: MapPoint::new(80.0, 20.0),
                target_after_movement: MapPoint::new(120.0, 20.0),
                target_position_observed_by_follow: MapPoint::new(120.0, 20.0),
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
    use crate::position_interface::SectorHandle;
    use crate::sequence::{MoveFlags, SequenceElement, SequenceElementData, SequenceState};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    #[derive(Debug, PartialEq)]
    struct Observation {
        seeker_slot: u32,
        target_slot: u32,
        target_before_movement: MapPoint,
        target_after_movement: MapPoint,
        seeker_state: SequenceState,
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

        // The original sprite pipeline reports MotionState::Start without
        // advancing on a newly-seen order. Prime that start tick, then use
        // the next production movement tick as the ordering observation.
        let assets = LevelAssets::new();
        engine.tick_entity_movement(sim, &assets);
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
            seeker_state: engine
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
                // The target turns one sector toward +X on this frame, so
                // the original 20-unit frame distance receives the 0.6 turn
                // slowdown and commits a 12-unit step.
                target_after_movement: MapPoint::new(22.0, 0.0),
                seeker_state: SequenceState::Terminated,
            },
            Observation {
                seeker_slot: 1,
                target_slot: 0,
                target_before_movement: MapPoint::new(10.0, 0.0),
                target_after_movement: MapPoint::new(22.0, 0.0),
                seeker_state: SequenceState::InProgress,
            },
        ],
        "a seeker before its target observes the pre-move position, while a seeker after its target observes the committed post-move position"
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
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .expect("movement remains inspectable")
            .state,
        SequenceState::InProgress,
        "a deflected final step must reconsider arrival on a later tick"
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

fn pending_specific_blinks(engine: &EngineInner, npc_id: EntityId) -> Vec<EntityId> {
    engine
        .get_entity(npc_id)
        .and_then(|entity| entity.ai_controller())
        .map(|ai| ai.outbox.actor.blink_enemy_specific.clone())
        .expect("NPC has AI controller")
}

#[test]
fn deferred_wakeup_pc_queues_specific_blink_for_opposite_camp_npcs() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::combat::ConcussionOutcome;
    use crate::element::{Camp, Posture};

    let mut engine = EngineInner::new();
    let waker = engine.add_entity(make_test_pc(Posture::Upright));
    let same_camp_npc = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let opposite_camp_npc = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

    engine
        .orders
        .pending_concussion_side_effects
        .push((waker, ConcussionOutcome::WokeUp));
    engine.drain_pending_concussion_side_effects(sim, &LevelAssets::new());

    assert_eq!(
        pending_specific_blinks(&engine, same_camp_npc),
        Vec::<EntityId>::new()
    );
    assert_eq!(
        pending_specific_blinks(&engine, opposite_camp_npc),
        vec![waker]
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

    engine
        .orders
        .pending_concussion_side_effects
        .push((waker, ConcussionOutcome::WokeUp));
    engine.drain_pending_concussion_side_effects(sim, &LevelAssets::new());

    assert_eq!(
        pending_specific_blinks(&engine, waker),
        Vec::<EntityId>::new()
    );
    assert_eq!(
        pending_specific_blinks(&engine, same_camp_npc),
        Vec::<EntityId>::new()
    );
    assert_eq!(
        pending_specific_blinks(&engine, opposite_camp_npc),
        Vec::<EntityId>::new(),
        "NPC wake blink must not fan out globally before the waker's creation slot"
    );
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

    engine
        .orders
        .pending_concussion_side_effects
        .push((waker, ConcussionOutcome::WokeUp));
    engine.drain_pending_concussion_side_effects(sim, &LevelAssets::new());

    assert_eq!(
        pending_specific_blinks(&engine, opposite_camp_npc),
        Vec::<EntityId>::new()
    );
}
