use super::*;

#[test]
fn actor_owner_envelope_closes_each_legacy_slot_before_the_next_owner() {
    use super::super::tick::{ActorOwnerEnvelopePhase as Phase, capture_actor_owner_envelope};

    let mut engine = EngineInner::new();
    let pc = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let npc = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();

    let (_, trace) = capture_actor_owner_envelope(|| {
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev)
    });

    assert_eq!(
        trace,
        vec![
            Phase::HumanPrelude(pc),
            Phase::BaseActor(pc),
            Phase::MovementExecute(pc),
            Phase::HumanNoise(pc),
            Phase::HumanTiredness(pc),
            Phase::PcTail(pc),
            Phase::SoldierPrelude(npc),
            Phase::Patrol(npc),
            Phase::HumanPrelude(npc),
            Phase::BaseActor(npc),
            Phase::MovementExecute(npc),
            Phase::HumanTiredness(npc),
            Phase::NpcTail(npc),
        ],
        "the complete PC envelope, including produced noise, must close before the following NPC begins"
    );
}

#[test]
fn listen_fires_on_25th_owner_invocation_with_strict_3d_cross_layer_scan() {
    use crate::element::{Command, ElementData, ElementKind, TargetFilter};
    use crate::movement::{AbilityKind, ActiveAbility};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let sim = crate::sim_rng::SimulationContext::with_seed_and_config(
        1,
        crate::engine::SimConfig {
            script_enabled: false,
            ..Default::default()
        },
    );
    let mut assets = LevelAssets::new();
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.characters.push(Default::default());
    assets.profile_manager = std::sync::Arc::new(profiles);
    let mut engine = EngineInner::new();
    let listener = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let near = engine.add_test_entity(make_discovery_bonus(450.0));
    let exact = engine.add_test_entity(make_discovery_bonus(450.0));
    let target = engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::Target;
            initial_element
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    engine
        .get_entity_mut(listener)
        .unwrap()
        .element_data_mut()
        .set_position(crate::coordinates::WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
    let near_element = engine.get_entity_mut(near).unwrap().element_data_mut();
    near_element.set_position_map(MapPoint::new(450.0, 0.0));
    near_element.set_layer(7);
    near_element.set_position(crate::coordinates::WorldPoint3D {
        x: 450.0,
        y: 0.0,
        z: 100.0,
    });
    let exact_element = engine.get_entity_mut(exact).unwrap().element_data_mut();
    exact_element.set_position_map(MapPoint::new(450.0, 0.0));
    exact_element.set_position(crate::coordinates::WorldPoint3D {
        x: 450.0,
        y: 0.0,
        z: 600.0,
    });
    let listener_z = engine
        .get_entity(listener)
        .unwrap()
        .element_data()
        .position()
        .z;
    let near_z = engine.get_entity(near).unwrap().element_data().position().z;
    let exact_z = engine
        .get_entity(exact)
        .unwrap()
        .element_data()
        .position()
        .z;
    assert_eq!(near_z - listener_z, 100.0, "inside case must exercise Z");
    assert_eq!(exact_z - listener_z, 600.0, "boundary case must exercise Z");
    assert!(450.0_f32.powi(2) + 100.0_f32.powi(2) < 750.0_f32.powi(2));
    assert_eq!(450.0_f32.powi(2) + 600.0_f32.powi(2), 750.0_f32.powi(2));
    let Entity::Target(target_entity) = engine.get_entity_mut(target).unwrap() else {
        unreachable!()
    };
    target_entity
        .target
        .action_filter
        .insert(TargetFilter::LISTEN);

    let mut element = SequenceElement::new(1, Command::EnterListen, Some(listener));
    let listening = Order::test_new(OrderType::Listening, 0.0, 0.0);
    let listening_id = listening.order_id;
    element.orders.push_back(listening);
    element.orders.push_back(Order::test_new(
        OrderType::TransitionListeningWaitingUpright,
        0.0,
        0.0,
    ));
    let seq = engine.orders.sequence_manager.launch_element(element);
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    let actor = engine
        .get_entity_mut(listener)
        .unwrap()
        .actor_data_mut()
        .unwrap();
    actor.listen_phase = crate::element::ListenPhase::CountingDown;
    actor.listen_wait_time = 0;
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::Listen),
        sequence_id: Some(seq),
        element_index: 0,
        target: None,
        order_id: Some(listening_id),
        done_effect_applied: false,
        strangle_initialized: false,
    };
    complete_test_runtime_fixture(&mut engine, &mut assets);

    // The listening animation deliberately ignores the sprite's completion
    // state until the 25-frame timer expires. Use a one-frame row here so a
    // generic ability tick would expose early order advancement immediately.
    let mut conversion =
        vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
    conversion[OrderType::Listening as usize] = 0;
    engine
        .get_entity_mut(listener)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
            action_id: OrderType::Listening as u16,
            action_done: 0,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        }]),
        std::sync::Arc::new(conversion),
    );
    let mut owner_driven = engine.clone();
    let mut owner_display = HostDisplayState::default();
    let mut owner_dev = DevState::default();
    for expected_wait in [24, 23, 22, 21] {
        owner_driven.perform_hourglass(
            &mut owner_display,
            &mut InputState::default(),
            &assets,
            &mut owner_dev,
        );
        let owner_actor = owner_driven
            .get_entity(listener)
            .unwrap()
            .actor_data()
            .unwrap();
        assert_eq!(owner_actor.listen_wait_time, expected_wait);
        assert_eq!(
            owner_actor.continuation.motion_state,
            crate::sprite::MotionState::InProgress,
            "PC execution must expose its Listening wrapper result, not the raw sprite edge"
        );
        assert_eq!(
            owner_actor.listen_phase,
            crate::element::ListenPhase::CountingDown,
            "Listening sprite completion must not advance the exit transition"
        );
        assert_eq!(
            owner_driven
                .orders
                .sequence_manager
                .get_element(seq, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .order_type,
            OrderType::Listening
        );
        assert_eq!(
            owner_driven
                .get_entity(listener)
                .unwrap()
                .element_data()
                .sprite
                .last_action,
            OrderType::Listening,
            "countdown must keep driving the visual action while ignoring its completion"
        );
    }

    for (case, frozen_all, execution_frozen, fried, expected) in [
        ("FrozenAll", true, false, false, 24),
        ("execution_frozen", false, true, false, 0),
        ("fried", false, false, true, 0),
    ] {
        let mut gated = engine.clone();
        gated.set_actors_frozen(frozen_all);
        let Entity::Pc(pc) = gated.get_entity_mut(listener).unwrap() else {
            unreachable!()
        };
        pc.actor.execution_frozen = execution_frozen;
        pc.pc.fried_psykokwack = fried;
        if frozen_all {
            // A non-zero phase makes an accidental Listening sprite tick
            // observable even on this fixture's one-frame animation.
            pc.element.sprite.frame_count = 7;
        }
        let mut display = HostDisplayState::default();
        let mut dev = DevState::default();
        gated.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
        assert_eq!(
            gated
                .get_entity(listener)
                .unwrap()
                .actor_data()
                .unwrap()
                .listen_wait_time,
            expected,
            "{case} owner gate"
        );
        if frozen_all {
            assert_eq!(
                gated
                    .get_entity(listener)
                    .unwrap()
                    .element_data()
                    .sprite
                    .frame_count,
                7,
                "FrozenAll must preserve the Listening sprite phase"
            );
        }
    }

    for invocation in 1..25 {
        assert!(!engine.tick_enemy_ai_blip_detection_for_owner(&sim, &assets, listener));
        assert_eq!(
            engine
                .get_entity(listener)
                .unwrap()
                .actor_data()
                .unwrap()
                .listen_wait_time,
            25 - invocation
        );
        assert!(engine.get_entity(near).unwrap().element_data().blipped);
    }
    assert!(engine.tick_enemy_ai_blip_detection_for_owner(&sim, &assets, listener));
    assert!(
        !engine.get_entity(near).unwrap().element_data().blipped,
        "450-100 strictly-near 3D cross-layer target reveals"
    );
    assert!(
        engine.get_entity(exact).unwrap().element_data().blipped,
        "450-600-750 exact 3D boundary remains out"
    );
    let Entity::Target(target_entity) = engine.get_entity(target).unwrap() else {
        unreachable!()
    };
    assert!(
        target_entity
            .target
            .action_filter
            .contains(TargetFilter::LISTEN),
        "scripts-disabled Heard retains LISTEN"
    );
    assert_eq!(
        engine
            .get_entity(listener)
            .unwrap()
            .actor_data()
            .unwrap()
            .listen_phase,
        crate::element::ListenPhase::ExitTransition
    );
}

#[test]
fn production_listen_creation_order_runs_heard_before_later_reveal_and_excludes_callback_append() {
    use crate::element::{Command, ElementData, ElementKind, TargetFilter};
    use crate::movement::{AbilityKind, ActiveAbility};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let (mut engine, target) = crate::engine::target_script_tests::build_engine_with_target();
    let listener = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let reveal = engine.add_test_entity(make_discovery_bonus(10.0));
    let Entity::Target(target_entity) = engine.get_entity_mut(target).unwrap() else {
        unreachable!()
    };
    target_entity.target.action_filter = TargetFilter::LISTEN;
    target_entity
        .element
        .set_position_map(MapPoint::new(20.0, 0.0));

    let mut element = SequenceElement::new(1, Command::EnterListen, Some(listener));
    let listening = Order::test_new(OrderType::Listening, 0.0, 0.0);
    let order_id = listening.order_id;
    element.orders.push_back(listening);
    element.orders.push_back(Order::test_new(
        OrderType::TransitionListeningWaitingUpright,
        0.0,
        0.0,
    ));
    let seq = engine.orders.sequence_manager.launch_element(element);
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    let actor = engine
        .get_entity_mut(listener)
        .unwrap()
        .actor_data_mut()
        .unwrap();
    actor.listen_phase = crate::element::ListenPhase::CountingDown;
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::Listen),
        sequence_id: Some(seq),
        element_index: 0,
        target: None,
        order_id: Some(order_id),
        done_effect_applied: false,
        strangle_initialized: false,
    };
    engine.set_actors_frozen(true);

    let observed_clear = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed_later_reveal_still_blipped = std::rc::Rc::new(std::cell::Cell::new(false));
    let appended = std::rc::Rc::new(std::cell::Cell::new(None));
    let observed_clear_hook = observed_clear.clone();
    let observed_later_reveal_hook = observed_later_reveal_still_blipped.clone();
    let appended_hook = appended.clone();
    crate::engine::ai::set_heard_callback_observer(Some(Box::new(move |engine, heard_target| {
        let Entity::Target(target) = engine.get_entity(heard_target).unwrap() else {
            unreachable!()
        };
        observed_clear_hook.set(!target.target.action_filter.contains(TargetFilter::LISTEN));
        observed_later_reveal_hook.set(
            engine
                .get_entity(reveal)
                .expect("later reveal entity exists during Heard callback")
                .element_data()
                .blipped,
        );
        let appended_id = engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::Target;
                initial_element
            },
            fx: Default::default(),
            target: crate::element::TargetData {
                action_filter: TargetFilter::LISTEN,
                script_class: "TestTarget".into(),
                ..Default::default()
            },
        }));
        appended_hook.set(Some(appended_id));
    })));

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    for _ in 0..25 {
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
    }
    crate::engine::ai::set_heard_callback_observer(None);

    assert!(!engine.get_entity(reveal).unwrap().element_data().blipped);
    assert!(
        observed_later_reveal_still_blipped.get(),
        "earlier Target Heard callback must run before the later-created reveal entity"
    );
    assert!(
        observed_clear.get(),
        "LISTEN must clear before the VM callback returns"
    );
    assert_eq!(
        crate::engine::target_script_tests::host_global(
            &engine,
            crate::engine::target_script_tests::GLOBAL_ID_HEARD
        ),
        crate::engine::target_script_tests::SENTINEL_HEARD
    );
    let appended = appended.get().expect("callback appended target");
    let Entity::Target(appended) = engine.get_entity(appended).unwrap() else {
        unreachable!()
    };
    assert!(
        appended.target.action_filter.contains(TargetFilter::LISTEN),
        "captured-length scan must exclude callback-appended entities"
    );
}

#[test]
fn tiredness_recovery_uses_original_creation_order_cadence() {
    let mut engine = EngineInner::new();
    let restored = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let aligned = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("tiredness fixtures have a character profile")
        .endurance = 90;

    for owner in [restored, aligned] {
        engine
            .get_entity_mut(owner)
            .and_then(Entity::human_data_mut)
            .expect("tiredness fixture remains human")
            .tiredness = 100;
    }

    let restored_order = (restored.index() + 17) & 31;
    engine
        .world
        .original_creation_order_by_entity
        .insert(restored, restored_order);
    engine
        .world
        .original_creation_order_by_entity
        .insert(aligned, aligned.index());

    engine.control.frame_counter = restored.index() & 31;
    engine.tick_tiredness_for(restored, &assets);
    assert_eq!(
        engine
            .get_entity(restored)
            .and_then(Entity::human_data)
            .expect("restored fixture remains human")
            .tiredness,
        100,
        "the kind-local entity slot must not open the recovered cadence"
    );

    engine.control.frame_counter = restored_order;
    engine.tick_tiredness_for(restored, &assets);
    assert_eq!(
        engine
            .get_entity(restored)
            .and_then(Entity::human_data)
            .expect("restored fixture remains human")
            .tiredness,
        91,
        "the restored Original creation-order slot subtracts endurance / 10"
    );

    engine.control.frame_counter = aligned.index() & 31;
    engine.tick_tiredness_for(aligned, &assets);
    assert_eq!(
        engine
            .get_entity(aligned)
            .and_then(Entity::human_data)
            .expect("aligned fixture remains human")
            .tiredness,
        91,
        "aligned entity and creation-order slots retain the existing behavior"
    );
}

#[test]
fn patrol_direction_macro_effect_closes_at_the_chief_owner_boundary() {
    use crate::ai::Substate;
    use crate::element::{ActionState, Camp, Entity};

    let mut engine = EngineInner::new();
    let chief = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let member = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    for id in [chief, member] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).unwrap() else {
            unreachable!()
        };
        soldier.element.active = true;
        soldier.npc.life_points = 100;
        soldier.npc.ai_brain.base_mut().unwrap().me = id.index();
    }
    let chief_ai = engine
        .get_entity_mut(chief)
        .unwrap()
        .ai_controller_mut()
        .unwrap();
    chief_ai.patrol = vec![member];
    chief_ai.instruct_patrol_direction_to_patrol_members(7);

    let Entity::Soldier(member_entity) = engine.get_entity_mut(member).unwrap() else {
        unreachable!()
    };
    member_entity.element.set_direction_instantly(3);
    member_entity.actor.action_state = ActionState::Waiting;
    member_entity
        .npc
        .ai_brain
        .base_mut()
        .unwrap()
        .current_substate = Substate::DefaultPatrolEnrouteWaiting;

    crate::sim_rng::with_seed(0x0A01_3D1A, |sim| {
        engine.drain_pending_for_npc(sim, chief, &assets)
    });

    let member_ai = engine.get_entity(member).unwrap().ai_controller().unwrap();
    assert_eq!(member_ai.patrol_direction, 7);
    // Facing launches a sequence; the Turn element becomes the actor's live
    // order only when the sequence manager promotes it, so the synchronous
    // observable at the chief macro boundary is the about-to-be-launched
    // Turn, not an already-current Turning order.
    assert!(
        engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(member, crate::element::Command::Turn),
        "CMD_PATROL_DIRECTION must synchronously enqueue the waiting member's facing turn before the chief macro boundary returns"
    );
}

#[test]
fn civilian_timer_retained_self_and_macro_boundaries_launch_orders_immediately() {
    use crate::ai::{AiState, Position, Stimulus, StimulusType, Substate};
    use crate::element::Command;

    fn add_ready_civilian(engine: &mut EngineInner) -> EntityId {
        let id = engine.add_test_entity(make_test_civilian(crate::element::Posture::Upright));
        let Entity::Civilian(civilian) = engine.get_entity_mut(id).expect("civilian exists") else {
            panic!("civilian changed kind")
        };
        civilian.element.active = true;
        civilian.npc.life_points = 100;
        civilian.element.set_position_map(MapPoint::new(0.0, 0.0));
        id
    }

    fn assert_drained(engine: &EngineInner, id: EntityId, boundary: &str) {
        let ai = engine
            .get_entity(id)
            .and_then(Entity::ai_controller)
            .expect("civilian retains AI");
        assert!(
            ai.outbox.actor.orders.is_empty(),
            "{boundary} must not leave civilian orders for a global batch"
        );
    }

    fn assert_launched(engine: &EngineInner, id: EntityId, expected: Command, boundary: &str) {
        assert_drained(engine, id, boundary);
        assert!(
            engine.actor_command(id) == expected
                || engine
                    .orders
                    .sequence_manager
                    .element_is_about_to_be_launched(id, expected),
            "{boundary} must synchronously launch or enqueue the civilian {expected:?} command"
        );
    }

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let timer_owner = add_ready_civilian(&mut engine);
    let retained_owner = add_ready_civilian(&mut engine);
    let self_owner = add_ready_civilian(&mut engine);
    let periodic_owner = add_ready_civilian(&mut engine);
    let macro_owner = add_ready_civilian(&mut engine);
    let target = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let Entity::Pc(pc) = engine.get_entity_mut(target).expect("face target exists") else {
        panic!("face target changed kind")
    };
    pc.element.active = true;
    pc.element.set_position_map(MapPoint::new(100.0, 0.0));
    pc.pc.life_points = 100;
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let timer_ai = engine
        .get_entity_mut(timer_owner)
        .and_then(Entity::ai_controller_mut)
        .expect("timer civilian has AI");
    timer_ai.current_state = AiState::Wondering;
    timer_ai.current_substate = Substate::WonderingCivilianAdmiringHero;
    timer_ai.initial_position = Position {
        x: 100.0,
        y: 0.0,
        ..Position::default()
    };
    timer_ai.timer_is_running = true;
    timer_ai.when_does_timer_ring = 0;
    timer_ai.substate_at_last_timer_launch = timer_ai.current_substate;
    engine.tick_ai_normal_timer_for_npc(sim, timer_owner, &assets);
    assert_drained(&engine, timer_owner, "normal timer");
    assert_eq!(
        engine
            .get_entity(timer_owner)
            .and_then(Entity::ai_controller)
            .expect("timer civilian retains AI")
            .current_substate,
        Substate::DefaultGotoPost,
        "normal timer Think must complete before the next owner"
    );

    for (id, queue) in [
        (retained_owner, Some(Stimulus::new(StimulusType::EventDone))),
        (self_owner, None),
    ] {
        let ai = engine
            .get_entity_mut(id)
            .and_then(Entity::ai_controller_mut)
            .expect("face civilian has AI");
        ai.current_state = AiState::Seeking;
        ai.current_substate = Substate::SeekingCivilianGiveAlertingReportToSoldierPoint;
        ai.antagonist = Some(crate::ai::AiEntityHandle::new(target.index()));
        if let Some(stimulus) = queue {
            ai.stimulus_queue.push(stimulus);
        } else {
            ai.outbox
                .reentrant
                .self_stimuli
                .push(StimulusType::EventDone.into());
        }
    }
    engine.tick_ai_queued_stimuli_for_npc(sim, retained_owner, &assets);
    assert_launched(&engine, retained_owner, Command::Turn, "retained Think");
    engine.drain_self_stimuli_for_npc(sim, self_owner, &assets);
    assert_launched(&engine, self_owner, Command::Turn, "recursive self-Think");

    let periodic_ai = engine
        .get_entity_mut(periodic_owner)
        .and_then(Entity::ai_controller_mut)
        .expect("The16thFrame civilian has AI");
    periodic_ai.current_state = AiState::Default;
    periodic_ai.current_substate = Substate::DefaultGotoPost;
    periodic_ai.stuck_counter = 3;
    periodic_ai.last_goto_destination = Position {
        x: 100.0,
        y: 0.0,
        ..Position::default()
    };
    // The periodic update's cadence is keyed on the NPC's register number (0 for
    // every fixture civilian), not its entity index: phase is
    // (frame & 255) - ((register + 100) & 255) and must be ≡ 0 mod 16.
    engine.control.frame_counter = 100;
    engine.tick_periodic_ai_for_npc(sim, periodic_owner, &assets);
    assert_drained(&engine, periodic_owner, "civilian The16thFrame");
    assert_eq!(
        engine
            .get_entity(periodic_owner)
            .and_then(Entity::ai_controller)
            .expect("The16thFrame civilian retains AI")
            .stuck_counter,
        0,
        "the periodic update must run and hand its movement retry to the engine boundary"
    );

    let macro_ai = engine
        .get_entity_mut(macro_owner)
        .and_then(Entity::ai_controller_mut)
        .expect("macro civilian has AI");
    macro_ai.current_state = AiState::Default;
    macro_ai.current_substate = Substate::DefaultInMacro;
    macro_ai.macro_command = vec![3, 8, 0]; // CMD_FACE_TO(8)
    macro_ai.macro_command_offset = 0;
    macro_ai.number_of_remaining_macro_bytes = 3;
    macro_ai.macro_timer_is_running = true;
    macro_ai.when_does_macro_timer_ring = 0;
    engine.tick_ai_macro_timer_for_npc(sim, macro_owner, &assets);
    assert_launched(&engine, macro_owner, Command::Turn, "macro VM");
}

#[test]
fn successful_patrol_dispatch_closes_chief_actor_boundary_before_returning() {
    use crate::ai::{AiState, CrossNpcAction, Position, StimulusInfo, StimulusType, Substate};
    use crate::element::{Camp, Entity};

    let mut engine = EngineInner::new();
    let chief_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let subordinate_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    for (id, x) in [(chief_id, 0.0), (subordinate_id, 10.0)] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).unwrap() else {
            panic!("patrol-dispatch test NPC changed kind")
        };
        soldier.element.active = true;
        soldier.element.set_position_map(MapPoint::new(x, 0.0));
        soldier.npc.life_points = 100;
        soldier.npc.view_radius = 400;
        soldier.npc.ai_brain.base_mut().unwrap().me = id.index();
    }
    {
        let chief = engine
            .get_entity_mut(chief_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap();
        chief.current_state = AiState::Default;
        chief.current_substate = Substate::DefaultOnPost;
        chief.patrol = vec![subordinate_id];
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .get_entity_mut(subordinate_id)
        .and_then(Entity::ai_controller_mut)
        .unwrap()
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::RequestPatrolDispatch {
            chief: chief_id.index(),
            caller: subordinate_id.index(),
            stimulus_type: StimulusType::EventSeesShadow,
            info: StimulusInfo::Position(Position {
                x: 100.0,
                y: 0.0,
                ..Position::default()
            }),
        });

    crate::sim_rng::with_seed(0xA013_2640, |sim| {
        engine.process_synchronous_reentrant_actions_for(sim, subordinate_id, &assets);
    });

    let chief = engine
        .get_entity(chief_id)
        .and_then(Entity::ai_controller)
        .unwrap();
    assert_eq!(chief.current_state, AiState::Default);
    assert_eq!(chief.current_substate, Substate::DefaultLookingShadow);
    assert!(
        !chief.outbox.actor.has_boundary_work(),
        "the direct chief routine must close its Halt/Face work before returning to the subordinate"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(chief_id, |command| {
                command == crate::element::Command::Turn
            }),
        "the chief's synchronous Face must register its Turn before the patrol call returns"
    );
}

#[test]
fn queued_fit_again_dispatches_at_owner_slot_for_soldiers_and_civilians() {
    use crate::ai::{AiState, StimulusType, Substate};
    use crate::element::{AiBrain, Camp, Entity, EyeStatus, Posture};

    for civilian in [false, true] {
        let mut engine = EngineInner::new();
        let entity = if civilian {
            let mut entity = make_test_civilian(Posture::Lying);
            let Entity::Civilian(civilian) = &mut entity else {
                unreachable!()
            };
            civilian.npc.ai_brain =
                AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(0)));
            entity
        } else {
            make_test_ai_soldier(Camp::Lacklandists)
        };
        let npc_id = engine.add_test_entity(entity);
        let mut assets = LevelAssets::new();
        // Install the active soldier profile before marking the actor
        // unconscious; the fixture intentionally skips unconscious soldiers.
        engine
            .get_entity_mut(npc_id)
            .unwrap()
            .element_data_mut()
            .active = true;
        complete_test_runtime_fixture(&mut engine, &mut assets);
        if !civilian {
            std::sync::Arc::make_mut(&mut assets.profile_manager).soldiers[0].wake_up = 1;
        }
        let entity = engine.get_entity_mut(npc_id).unwrap();
        entity
            .element_data_mut()
            .publish_order_posture(Posture::Lying);
        let human = entity.human_data_mut().unwrap();
        human.unconscious = true;
        human.concussion_of_the_brain = crate::combat::CONCUSSION_WAKEUP_THRESHOLD;
        human.concussion_healing_timeout = 0;
        let npc = entity.npc_data_mut().unwrap();
        npc.life_points = 100;
        npc.eye_status = EyeStatus::Closed;
        npc.view_radius = 0;
        npc.view_radius_base = 173;
        npc.view_radius_goal = 173;
        npc.view_longrange_radius_factor = 1.0;
        let ai = npc.ai_brain.base_mut().unwrap();
        ai.me = npc_id.index();
        ai.script_locked = false;
        ai.current_state = AiState::Sleeping;
        ai.current_substate = Substate::SleepingUnconscious;

        engine.tick_concussion_healing(&assets);

        let entity = engine.get_entity(npc_id).unwrap();
        assert!(!entity.human_data().unwrap().unconscious);
        assert_eq!(entity.element_data().posture(), Posture::Lying);
        assert_eq!(entity.npc_data().unwrap().eye_status, EyeStatus::Closed);
        assert_eq!(entity.npc_data().unwrap().view_radius, 0);
        assert_eq!(
            entity
                .ai_controller()
                .unwrap()
                .outbox
                .detection
                .stimuli
                .iter()
                .map(|stimulus| stimulus.stimulus_type)
                .collect::<Vec<_>>(),
            vec![StimulusType::EventFitAgain]
        );

        let mut positions = engine.boundary_positions_snapshot();
        crate::sim_rng::with_seed(0x0A01_3F17, |sim| {
            engine.tick_enemy_ai_with_creation_ordered_prelude(sim, &assets, &positions)
        });

        let entity = engine.get_entity(npc_id).unwrap();
        let ai = entity.ai_controller().unwrap();
        assert_ne!(ai.current_substate, Substate::SleepingUnconscious);
        assert_eq!(
            entity.npc_data().unwrap().eye_status,
            EyeStatus::LookForward
        );
        assert_eq!(
            entity.npc_data().unwrap().view_radius,
            173,
            "owner-slot recovery must open the eyes before that NPC refreshes its view"
        );
        assert!(ai.outbox.detection.stimuli.is_empty());
        assert!(!ai.outbox.recovery.inform_resurrection);
        assert_eq!(ai.outbox.recovery.set_eye_status, None);
    }
}

#[test]
fn playable_rescue_pc_without_command_interface_still_sees_blips() {
    use crate::element::{Camp, Entity};
    use crate::human_control::{CommandInterface, MissionRole};

    let mut engine = EngineInner::new();
    // Slot 0 has Original creation order 31; frame 1 opens the common
    // modulo-16 blip cadence.
    engine.control.frame_counter = 1;
    let observer_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("blipped observer exists")
    else {
        panic!("blipped observer changed kind")
    };
    observer.element.active = true;
    observer.element.blipped = true;
    observer.npc.life_points = 100;
    observer
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(20.0, 0.0, 0.0));
    observer.element.set_position_map(MapPoint::new(20.0, 0.0));

    let Entity::Pc(pc) = engine.get_entity_mut(pc_id).expect("rescue PC exists") else {
        panic!("rescue PC changed kind")
    };
    pc.element.active = true;
    pc.pc.playable = true;
    pc.pc.command_interface = CommandInterface::None;
    pc.pc.mission_role = MissionRole::RescueTarget;
    pc.pc.life_points = 100;
    pc.element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    pc.element.set_position_map(MapPoint::new(0.0, 0.0));

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    crate::sight_obstacle::begin_parity_visibility_capture();
    crate::sim_rng::with_seed(0xA013_B11F, |sim| engine.tick_enemy_ai(sim, &assets));
    let queries = crate::sight_obstacle::take_parity_visibility_capture();

    assert_eq!(
        queries.len(),
        1,
        "Original SeesBlip is independent of the player's command interface"
    );
    assert!(queries[0].result);
    assert!(
        !engine
            .get_entity(observer_id)
            .expect("blipped observer survives tick")
            .element_data()
            .blipped,
        "the playable rescue PC must reveal the nearby blip"
    );
}

#[test]
fn bonus_refresh_discovered_observes_owner_callback_order_and_spawned_later_slots() {
    fn observed(pc_first: bool) -> (bool, bool) {
        let mut engine = EngineInner::new();
        engine.ai.standard_view_polygon_radius = 100;
        let (pc_id, bonus_id) = if pc_first {
            let pc = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
            let bonus = engine.add_test_entity(make_discovery_bonus(10.0));
            (pc, bonus)
        } else {
            let bonus = engine.add_test_entity(make_discovery_bonus(10.0));
            let pc = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
            (pc, bonus)
        };
        let Entity::Pc(pc) = engine.get_entity_mut(pc_id).unwrap() else {
            unreachable!()
        };
        pc.element.active = false;
        pc.pc.life_points = 100;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(5_000.0, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(5_000.0, 0.0));
        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let mut positions = engine.boundary_positions_snapshot();
        let mut spawned = None;
        crate::sim_rng::with_seed(0x0B0A_00CB, |sim| {
            engine.tick_actor_owner_envelopes_with_test_owner_hook(
                sim,
                &assets,
                &positions,
                |engine, owner| {
                    if owner != pc_id {
                        return;
                    }
                    let Entity::Pc(pc) = engine.get_entity_mut(pc_id).unwrap() else {
                        unreachable!()
                    };
                    pc.element.active = true;
                    pc.element
                        .publish_order_posture(crate::element::Posture::OnShoulders);
                    pc.element
                        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
                    pc.element.set_position_map(MapPoint::new(0.0, 0.0));
                    spawned = Some(engine.add_test_entity(make_discovery_bonus(10.0)));
                },
            );
        });
        (
            !engine.get_entity(bonus_id).unwrap().element_data().blipped,
            !engine
                .get_entity(spawned.expect("PC callback spawned a later bonus"))
                .unwrap()
                .element_data()
                .blipped,
        )
    }

    assert_eq!(observed(true), (true, true));
    assert_eq!(observed(false), (false, true));
}
