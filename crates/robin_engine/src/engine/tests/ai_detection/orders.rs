use super::*;
use crate::engine::TickCtx;

#[test]
fn actor_owner_envelope_closes_each_legacy_slot_before_the_next_owner() {
    use super::super::tick::{ActorOwnerEnvelopePhase as Phase, capture_actor_owner_envelope};

    let mut engine = EngineInner::new();
    let pc = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let npc = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let assets = engine.test_runtime_assets();
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
    engine.place(
        listener,
        crate::coordinates::WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    );
    let near_element = engine.elem_mut(near);
    near_element.set_position_map(MapPoint::new(450.0, 0.0));
    near_element.set_layer(7);
    near_element.set_position(crate::coordinates::WorldPoint3D {
        x: 450.0,
        y: 0.0,
        z: 100.0,
    });
    let exact_element = engine.elem_mut(exact);
    exact_element.set_position_map(MapPoint::new(450.0, 0.0));
    exact_element.set_position(crate::coordinates::WorldPoint3D {
        x: 450.0,
        y: 0.0,
        z: 600.0,
    });
    let listener_z = engine.pos_of(listener).z;
    let near_z = engine.pos_of(near).z;
    let exact_z = engine.pos_of(exact).z;
    assert_eq!(near_z - listener_z, 100.0, "inside case must exercise Z");
    assert_eq!(exact_z - listener_z, 600.0, "boundary case must exercise Z");
    assert!(450.0_f32.powi(2) + 100.0_f32.powi(2) < 750.0_f32.powi(2));
    assert_eq!(450.0_f32.powi(2) + 600.0_f32.powi(2), 750.0_f32.powi(2));
    let Entity::Target(target_entity) = engine.ent_mut(target) else {
        unreachable!()
    };
    target_entity
        .target
        .action_filter
        .insert(TargetFilter::LISTEN);

    let mut element = SequenceElement::new(1, Command::EnterListen, Some(listener));
    let listening = Order::test_new(OrderType::Listening, 0.0, 0.0);
    element.orders.push_back(listening);
    element.orders.push_back(Order::test_new(
        OrderType::TransitionListeningWaitingUpright,
        0.0,
        0.0,
    ));
    let seq = engine.orders.sequence_manager.insert_element(element);
    engine.orders.sequence_manager.start_sequence_level(seq);
    engine.select_sequence_element(listener, Some((seq, 0)));
    engine.t_element_in_progress(&assets, seq, 0);
    let actor = engine.actor_mut(listener);
    actor.wait_time = 0;
    complete_test_runtime_fixture(&mut engine, &mut assets);

    // The listening animation deliberately ignores the sprite's completion
    // state until the 25-frame timer expires. Use a one-frame row here so a
    // generic ability tick would expose early order advancement immediately.
    let mut conversion = crate::engine::test_support::unmapped_conversion();
    conversion[OrderType::Listening as usize] = 0;
    engine.elem_mut(listener).sprite = crate::sprite::Sprite::new(
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
        let owner_actor = owner_driven.actor(listener);
        assert_eq!(owner_actor.wait_time, expected_wait);
        assert_eq!(
            owner_actor.continuation.motion_state,
            crate::sprite::MotionState::InProgress,
            "PC execution must expose its Listening wrapper result, not the raw sprite edge"
        );
        assert_eq!(
            owner_driven
                .orders
                .sequence_manager
                .current_order_for_actor(&owner_driven.world.entities, listener)
                .map(|(_, _, order)| order.order_type),
            Some(OrderType::Listening),
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
            owner_driven.elem(listener).sprite.last_action,
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
        let Entity::Pc(pc) = gated.ent_mut(listener) else {
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
            gated.actor(listener).wait_time,
            expected,
            "{case} owner gate"
        );
        if frozen_all {
            assert_eq!(
                gated.elem(listener).sprite.frame_count,
                7,
                "FrozenAll must preserve the Listening sprite phase"
            );
        }
    }

    // Direct Execute probes supply the initialization edge normally published
    // by owner selection. A restored zero timer does not restart or reveal.
    let mut restored = engine.clone();
    restored.actor_mut(listener).execute_order_initialising = false;
    assert_eq!(
        restored.tick_enemy_ai_blip_detection_for_owner(TickCtx::new(&sim, &assets), listener),
        Some(crate::sprite::MotionState::InProgress)
    );
    assert_eq!(restored.actor(listener).wait_time, 0);
    assert!(restored.elem(near).blipped);

    for invocation in 1..25 {
        engine.actor_mut(listener).execute_order_initialising = invocation == 1;
        assert_eq!(
            engine.tick_enemy_ai_blip_detection_for_owner(TickCtx::new(&sim, &assets), listener),
            Some(crate::sprite::MotionState::InProgress)
        );
        assert_eq!(engine.actor(listener).wait_time, 25 - invocation);
        assert!(engine.elem(near).blipped);
    }
    assert_eq!(
        engine.tick_enemy_ai_blip_detection_for_owner(TickCtx::new(&sim, &assets), listener),
        Some(crate::sprite::MotionState::Terminated)
    );
    assert!(
        !engine.elem(near).blipped,
        "450-100 strictly-near 3D cross-layer target reveals"
    );
    assert!(
        engine.elem(exact).blipped,
        "450-600-750 exact 3D boundary remains out"
    );
    let Entity::Target(target_entity) = engine.ent(target) else {
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
            .orders
            .sequence_manager
            .current_order_for_actor(&engine.world.entities, listener)
            .map(|(_, _, order)| order.order_type),
        Some(OrderType::Listening),
        "the returned terminal motion is advanced by the owner coordinator"
    );
}

#[test]
fn production_listen_creation_order_runs_heard_before_later_reveal() {
    let mut assets = LevelAssets::new();
    use crate::element::{Command, TargetFilter};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let (mut engine, target) = crate::engine::target_script_tests::build_engine_with_target();
    let listener = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let reveal = engine.add_test_entity(make_discovery_bonus(10.0));
    let Entity::Target(target_entity) = engine.ent_mut(target) else {
        unreachable!()
    };
    target_entity.target.action_filter = TargetFilter::LISTEN;
    target_entity
        .element
        .set_position_map(MapPoint::new(20.0, 0.0));

    let mut element = SequenceElement::new(1, Command::EnterListen, Some(listener));
    let listening = Order::test_new(OrderType::Listening, 0.0, 0.0);
    element.orders.push_back(listening);
    element.orders.push_back(Order::test_new(
        OrderType::TransitionListeningWaitingUpright,
        0.0,
        0.0,
    ));
    let seq = engine.orders.sequence_manager.insert_element(element);
    engine.orders.sequence_manager.start_sequence_level(seq);
    engine.select_sequence_element(listener, Some((seq, 0)));
    engine.t_element_in_progress(&assets, seq, 0);
    engine.actor_mut(listener);
    engine.set_actors_frozen(true);

    assets = engine.test_runtime_assets();
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let ((), heard) = crate::engine::ai::capture_heard_callbacks(|| {
        for _ in 0..25 {
            engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
        }
    });

    assert!(!engine.elem(reveal).blipped);
    let [heard] = heard.as_slice() else {
        panic!("expected exactly one Heard callback, got {heard:?}");
    };
    assert_eq!(heard.target, target);
    assert!(
        heard.blipped.contains(&reveal),
        "earlier Target Heard callback must run before the later-created reveal entity"
    );
    assert!(
        heard.listen_cleared,
        "LISTEN must clear before the VM callback returns"
    );
    assert_eq!(
        crate::engine::target_script_tests::host_global(
            &engine,
            crate::engine::target_script_tests::GLOBAL_ID_HEARD
        ),
        crate::engine::target_script_tests::SENTINEL_HEARD
    );
    // TODO: the captured-length guarantee (a target appended during the Heard
    // callback is not scanned) lost its coverage when the mid-tick mutating
    // observer was removed. No production path or test-script native can
    // append an entity inside ActivatedByListenable; restore the check once a
    // script-reachable entity-creating native exists in the test SCB.
}

#[test]
fn tiredness_recovery_uses_original_creation_order_cadence() {
    let mut engine = EngineInner::new();
    let restored = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let aligned = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let mut assets = engine.test_runtime_assets();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("tiredness fixtures have a character profile")
        .endurance = 90;

    for owner in [restored, aligned] {
        engine.human_mut(owner).tiredness = 100;
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
        engine.human(restored).tiredness,
        100,
        "the kind-local entity slot must not open the recovered cadence"
    );

    engine.control.frame_counter = restored_order;
    engine.tick_tiredness_for(restored, &assets);
    assert_eq!(
        engine.human(restored).tiredness,
        91,
        "the restored Original creation-order slot subtracts endurance / 10"
    );

    engine.control.frame_counter = aligned.index() & 31;
    engine.tick_tiredness_for(aligned, &assets);
    assert_eq!(
        engine.human(aligned).tiredness,
        91,
        "aligned entity and creation-order slots retain the existing behavior"
    );
}

#[test]
fn patrol_direction_instruction_registers_member_turn_before_returning() {
    use crate::ai::Substate;
    use crate::element::{ActionState, Camp, Entity};

    let mut engine = EngineInner::new();
    let chief = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let member = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let assets = engine.test_runtime_assets();

    for id in [chief, member] {
        let Entity::Soldier(soldier) = engine.ent_mut(id) else {
            unreachable!()
        };
        soldier.element.active = true;
        soldier.npc.life_points = 100;
        soldier.npc.ai_brain.base_mut().unwrap().me = id.index();
    }
    let chief_ai = engine.ai_ctrl_mut(chief);
    chief_ai.patrol = vec![member];

    let Entity::Soldier(member_entity) = engine.ent_mut(member) else {
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
        engine.instruct_patrol_direction_to_patrol_members(TickCtx::new(sim, &assets), chief, 7)
    });

    let member_ai = engine.ai_ctrl(member);
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
        let Entity::Civilian(civilian) = engine.ent_mut(id) else {
            panic!("civilian changed kind")
        };
        civilian.element.active = true;
        civilian.npc.life_points = 100;
        civilian.element.set_position_map(MapPoint::new(0.0, 0.0));
        id
    }

    fn assert_launched(engine: &EngineInner, id: EntityId, expected: Command, boundary: &str) {
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
    let Entity::Pc(pc) = engine.ent_mut(target) else {
        panic!("face target changed kind")
    };
    pc.element.active = true;
    pc.element.set_position_map(MapPoint::new(100.0, 0.0));
    pc.pc.life_points = 100;
    let assets = engine.test_runtime_assets();

    let timer_ai = engine.ai_ctrl_mut(timer_owner);
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
    engine.tick_ai_normal_timer_for_npc(TickCtx::new(sim, &assets), timer_owner);
    assert_eq!(
        engine.ai_ctrl(timer_owner).current_substate,
        Substate::DefaultGotoPost,
        "normal timer Think must complete before the next owner"
    );

    for (id, queue) in [
        (retained_owner, Some(Stimulus::new(StimulusType::EventDone))),
        (self_owner, None),
    ] {
        let ai = engine.ai_ctrl_mut(id);
        ai.current_state = AiState::Seeking;
        ai.current_substate = Substate::SeekingCivilianGiveAlertingReportToSoldierPoint;
        ai.antagonist = Some(crate::ai::AiEntityHandle::new(target.index()));
        if let Some(stimulus) = queue {
            ai.stimulus_queue.push(stimulus);
        }
    }
    engine.tick_ai_queued_stimuli_for_npc(TickCtx::new(sim, &assets), retained_owner);
    assert_launched(&engine, retained_owner, Command::Turn, "retained Think");
    engine.execute_ai_callback(
        TickCtx::new(sim, &assets),
        self_owner,
        &Stimulus::new(StimulusType::EventDone),
    );

    assert_launched(&engine, self_owner, Command::Turn, "recursive self-Think");

    let periodic_ai = engine.ai_ctrl_mut(periodic_owner);
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
    engine.tick_periodic_ai_for_npc(TickCtx::new(sim, &assets), periodic_owner);
    assert_eq!(
        engine.ai_ctrl(periodic_owner).stuck_counter,
        0,
        "the periodic update must run and hand its movement retry to the engine boundary"
    );

    let macro_ai = engine.ai_ctrl_mut(macro_owner);
    macro_ai.current_state = AiState::Default;
    macro_ai.current_substate = Substate::DefaultInMacro;
    macro_ai.macro_command = vec![3, 8, 0]; // CMD_FACE_TO(8)
    macro_ai.macro_command_offset = 0;
    macro_ai.number_of_remaining_macro_bytes = 3;
    macro_ai.macro_timer_is_running = true;
    macro_ai.when_does_macro_timer_ring = 0;
    engine.tick_ai_macro_timer_for_npc(TickCtx::new(sim, &assets), macro_owner);
    assert_launched(&engine, macro_owner, Command::Turn, "macro VM");
}

#[test]
fn successful_patrol_dispatch_closes_chief_actor_boundary_before_returning() {
    use crate::ai::{AiState, Position, Stimulus, StimulusInfo, StimulusType, Substate};
    use crate::element::{Camp, Entity};

    let mut engine = EngineInner::new();
    let chief_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let subordinate_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    for (id, x) in [(chief_id, 0.0), (subordinate_id, 10.0)] {
        let Entity::Soldier(soldier) = engine.ent_mut(id) else {
            panic!("patrol-dispatch test NPC changed kind")
        };
        soldier.element.active = true;
        soldier.element.set_position_map(MapPoint::new(x, 0.0));
        soldier.npc.life_points = 100;
        soldier.npc.view_radius = 400;
        soldier.npc.ai_brain.base_mut().unwrap().me = id.index();
    }
    {
        let chief = engine.ai_ctrl_mut(chief_id);
        chief.current_state = AiState::Default;
        chief.current_substate = Substate::DefaultOnPost;
        chief.patrol = vec![subordinate_id];
    }

    let assets = engine.test_runtime_assets();
    engine.ai_ctrl_mut(subordinate_id).patrol_chief = Some(chief_id);
    let mut stimulus = Stimulus::new(StimulusType::EventSeesShadow);
    stimulus.info = StimulusInfo::Position(Position {
        x: 100.0,
        y: 0.0,
        ..Position::default()
    });

    crate::sim_rng::with_seed(0xA013_2640, |sim| {
        engine.dispatch_filtered_stimulus(TickCtx::new(sim, &assets), subordinate_id, &stimulus);
    });

    let chief = engine.ai_ctrl(chief_id);
    assert_eq!(chief.current_state, AiState::Default);
    assert_eq!(chief.current_substate, Substate::DefaultLookingShadow);
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
fn natural_recovery_finishes_inline_for_soldiers_and_civilians() {
    use crate::ai::{AiState, Substate};
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
        // Install the active soldier profile before marking the actor
        // unconscious; the fixture intentionally skips unconscious soldiers.
        engine.set_active(npc_id, true);
        let mut assets = engine.test_runtime_assets();
        if !civilian {
            std::sync::Arc::make_mut(&mut assets.profile_manager).soldiers[0].wake_up = 1;
        }
        let entity = engine.ent_mut(npc_id);
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

        crate::sim_rng::with_seed(0x0A01_3F17, |sim| {
            engine.tick_concussion_healing_for(TickCtx::new(sim, &assets), npc_id)
        });

        let entity = engine.ent(npc_id);
        assert!(!entity.human_data().unwrap().unconscious);
        let ai = entity.ai_controller().unwrap();
        assert_ne!(ai.current_substate, Substate::SleepingUnconscious);
        assert_eq!(
            entity.npc_data().unwrap().eye_status,
            EyeStatus::LookForward
        );
        assert_eq!(
            entity.npc_data().unwrap().view_radius,
            0,
            "the recovery callback changes eye status before the view refresh updates its radius"
        );
        assert!(entity.npc_data().unwrap().view_transition);
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

    let Entity::Soldier(observer) = engine.ent_mut(observer_id) else {
        panic!("blipped observer changed kind")
    };
    observer.element.active = true;
    observer.element.blipped = true;
    observer.npc.life_points = 100;
    observer
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(20.0, 0.0, 0.0));
    observer.element.set_position_map(MapPoint::new(20.0, 0.0));

    let Entity::Pc(pc) = engine.ent_mut(pc_id) else {
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

    let assets = engine.test_runtime_assets();

    crate::sight_obstacle::begin_parity_visibility_capture();
    crate::sim_rng::with_seed(0xA013_B11F, |sim| {
        engine.tick_enemy_ai(TickCtx::new(sim, &assets))
    });
    let queries = crate::sight_obstacle::take_parity_visibility_capture();

    assert_eq!(
        queries.len(),
        1,
        "Original SeesBlip is independent of the player's command interface"
    );
    assert!(queries[0].result);
    assert!(
        !engine.elem(observer_id).blipped,
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
        let Entity::Pc(pc) = engine.ent_mut(pc_id) else {
            unreachable!()
        };
        pc.element.active = false;
        pc.pc.life_points = 100;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(5_000.0, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(5_000.0, 0.0));
        let assets = engine.test_runtime_assets();

        let mut spawned = None;
        crate::sim_rng::with_seed(0x0B0A_00CB, |sim| {
            engine.tick_actor_owner_envelopes_with_test_owner_hook(
                TickCtx::new(sim, &assets),
                |engine, owner| {
                    if owner != pc_id {
                        return;
                    }
                    let Entity::Pc(pc) = engine.ent_mut(pc_id) else {
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
            !engine.elem(bonus_id).blipped,
            !engine
                .elem(spawned.expect("PC callback spawned a later bonus"))
                .blipped,
        )
    }

    assert_eq!(observed(true), (true, true));
    assert_eq!(observed(false), (false, true));
}
