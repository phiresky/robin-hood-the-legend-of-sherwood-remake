use super::*;

#[test]
fn phalanx_them_list_snapshot_follows_inactive_linked_member() {
    use crate::element::{Camp, Entity};

    let mut engine = EngineInner::new();
    engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let chief = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let linked = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

    for id in [chief, linked] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).unwrap() else {
            unreachable!("test phalanx member changed kind")
        };
        soldier.npc.ai_brain.base_mut().unwrap().me = id.index();
    }
    let chief_ai = engine
        .get_entity_mut(chief)
        .and_then(Entity::enemy_ai_mut)
        .unwrap();
    chief_ai.right_combat_neighbour = Some(crate::ai::AiEntityHandle::new(linked.index()));

    let Entity::Soldier(linked_soldier) = engine.get_entity_mut(linked).unwrap() else {
        unreachable!()
    };
    linked_soldier.element.active = false;
    linked_soldier.human.unconscious = true;
    linked_soldier.npc.life_points = 0;

    let snapshots = engine.build_phalanx_member_them_lists(chief);
    assert_eq!(
        snapshots
            .iter()
            .map(|member| member.handle)
            .collect::<Vec<_>>(),
        [chief.index(), linked.index()],
        "Original follows the installed right-neighbour link without active, consciousness, or life guards"
    );
}

#[test]
fn primary_target_tracking_precedes_view_refresh() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();

    let soldier_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
    let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let soldier_pos = MapPoint::new(100.0, 100.0);
    let target_pos = MapPoint::new(100.0, 200.0);

    if let Some(Entity::Soldier(soldier)) = engine.get_entity_mut(soldier_id) {
        soldier.element.active = true;
        soldier.element.set_position_map(soldier_pos);
        soldier.element.set_direction_instantly(4);
        soldier.npc.direction_old = 4;
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test soldier has enemy AI");
        ai.base.me = soldier_id.index();
        ai.base.primary_target = Some(crate::ai::AiEntityHandle::new(pc_id.index()));
        ai.base.current_state = crate::ai::AiState::Attacking;
        ai.base.current_substate = crate::ai::Substate::AttackingReactiontime;
    }
    if let Some(Entity::Pc(pc)) = engine.get_entity_mut(pc_id) {
        pc.element.active = true;
        pc.element.set_position_map(target_pos);
    }

    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);

    let expected = crate::position_interface::vector_to_sector_0_to_15_iso(
        target_pos.x - soldier_pos.x,
        target_pos.y - soldier_pos.y,
    );
    let Entity::Soldier(soldier) = engine.get_entity(soldier_id).unwrap() else {
        panic!("test soldier changed entity kind");
    };
    assert_eq!(soldier.element.direction(), expected);
    assert_eq!(
        soldier.npc.direction_old, expected,
        "RefreshView must observe the combat tracking direction in the same frame"
    );
}

#[test]
fn npc_hourglass_observes_exact_original_phase_order() {
    use super::tick::{NpcHourglassPhase as Phase, capture_npc_hourglass_phases};

    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();

    let (_, phases) = capture_npc_hourglass_phases(|| {
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev)
    });

    assert_eq!(
        phases,
        vec![
            Phase::SoldierPrelude,
            Phase::Patrol,
            Phase::BaseHuman,
            Phase::Broadcasts,
            Phase::View,
            Phase::Detection,
            Phase::Ambush,
            Phase::Busy,
            Phase::Ladder,
            Phase::LockGate,
            Phase::SixteenthFrame,
            Phase::NormalTimer,
            Phase::MacroTimer,
            Phase::QueuedStimuli,
        ]
    );
}

#[test]
fn actor_owner_envelope_closes_each_legacy_slot_before_the_next_owner() {
    use super::tick::{ActorOwnerEnvelopePhase as Phase, capture_actor_owner_envelope};

    let mut engine = EngineInner::new();
    let pc = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let npc = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
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
fn periodic_bored_roll_reads_installed_order_after_detection_boundary() {
    use crate::element::InstalledActorOrder;
    use crate::order::OrderType;
    use crate::sim_rng::{RngSite, with_draw_trace};

    let mut engine = EngineInner::new();
    let npc_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    // Model a synchronous detection Think replacing mpOrder after Actor::Hourglass
    // selected its tail order. The sequence manager deliberately has no selected
    // order: only ActorData::installed_order mirrors Original's live mpOrder.
    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(npc_id)
            .is_none()
    );
    engine
        .get_entity_mut(npc_id)
        .and_then(Entity::actor_data_mut)
        .expect("periodic owner has actor data")
        .installed_order = Some(InstalledActorOrder {
        order_id: std::num::NonZeroU32::new(1).unwrap(),
        order_type: OrderType::WaitingUprightBored,
    });
    // Register zero reaches The16thFrame at frame 100.
    engine.control.frame_counter = 100;

    let sim = crate::sim_rng::test_context();
    let (_, trace) = with_draw_trace(|| engine.tick_periodic_ai_for_npc(&sim, npc_id, &assets));

    assert!(
        trace.contains(&RngSite::VipIdleRemark),
        "The16thFrame GetAnimation must read the installed mpOrder at its own boundary"
    );
}

#[test]
fn periodic_enemy_post_refresh_reads_the_materialized_manager_queue_without_surfacing_completion() {
    use crate::ai::{AiContext, AiState, GotoFlags, Position, Substate};
    use crate::element::{Camp, Command, Entity};
    use crate::order::OrderType;
    use crate::position_interface::SectorHandle;

    let sim = crate::sim_rng::test_context();
    let mut assets = LevelAssets::new();

    let mut run_case = |case: &str| {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
        complete_test_runtime_fixture(&mut engine, &mut assets);

        let position = Position {
            x: 100.0,
            y: 100.0,
            sector: SectorHandle::new(0),
            ..Position::default()
        };
        let ctx = AiContext {
            position,
            self_animation: OrderType::WaitingAlerted,
            self_is_soldier: true,
            ..AiContext::default()
        };
        let Entity::Soldier(soldier) = engine.get_entity_mut(owner).unwrap() else {
            unreachable!()
        };
        soldier
            .element
            .set_position_map(MapPoint::new(position.x, position.y));
        soldier.element.set_sector(position.sector);
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = owner.index();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunningToPhalanx;
        ai.base.stuck_counter = 2;
        // An enclosing Think owns this latch. The prefix boundary must not
        // manufacture its EndThink while materializing the GoTo registration.
        ai.base.think_recursion_depth = 1;
        ai.base.completion_latch_inside_think = true;
        if case == "accepted" {
            ai.base.open_end_think_frames = 1;
            ai.base.engine_deferred_end_think_frames = 1;
            ai.base.engine_completion_verdict_resolved = false;
        }
        let destination = match case {
            "accepted" => Position {
                x: 140.0,
                ..position
            },
            "already" => position,
            "denied" => Position {
                x: 140.0,
                sector: None,
                ..position
            },
            _ => unreachable!(),
        };
        ai.base.go_to(destination, GotoFlags::RUN, &ctx);

        engine.finish_enemy_periodic_stuck_suffix_after_refresh(&sim, owner, &assets, 0, &ctx);
        let pending = engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(owner, Command::Null);
        let ai = engine.get_entity(owner).and_then(Entity::enemy_ai).unwrap();
        (
            pending,
            ai.base.stuck_counter,
            ai.base.completion_latch_inside_think,
            ai.base.already_on_point,
            ai.base.couldnt_reachpoint,
            ai.base.think_recursion_depth,
            ai.base.open_end_think_frames,
            ai.base.engine_deferred_end_think_frames,
            ai.base.engine_completion_verdict_resolved,
        )
    };

    assert_eq!(
        run_case("accepted"),
        (true, 0, true, false, false, 1, 1, 1, true),
        "an accepted GoTo must register before the wildcard query, reset the watchdog, and retain the enclosing completion latch"
    );
    assert_eq!(
        run_case("already"),
        (false, 3, true, true, false, 1, 0, 0, false),
        "an already-on-point GoTo leaves no manager element, so the selected Wait advances without surfacing completion"
    );
    assert_eq!(
        run_case("denied"),
        (false, 3, true, false, true, 1, 0, 0, false),
        "a denied GoTo leaves no manager element, so the selected Wait advances without surfacing completion"
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
    let listener = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let near = engine.add_entity(make_discovery_bonus(450.0));
    let exact = engine.add_entity(make_discovery_bonus(450.0));
    let target = engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: ElementData {
            kind: ElementKind::Target,
            ..Default::default()
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

    // RHANIMATION_LISTENING deliberately ignores the sprite's completion
    // state until the 25-frame timer expires. Use a one-frame row here so a
    // generic ability tick would expose an early DoNextOrder immediately.
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
            "PC::Execute must expose its Listening wrapper result, not the raw sprite edge"
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
    let listener = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let reveal = engine.add_entity(make_discovery_bonus(10.0));
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
        let appended_id = engine.add_entity(Entity::Target(crate::element::ElementTarget {
            element: ElementData {
                kind: ElementKind::Target,
                ..Default::default()
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
fn pc_noise_is_live_at_the_following_npc_slot_only() {
    use crate::element::{Camp, Detectable, DetectableType};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    fn observe(pc_first: bool) -> bool {
        let mut engine = EngineInner::new();
        let (pc, npc) = if pc_first {
            let pc = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
            let npc = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
            (pc, npc)
        } else {
            let npc = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
            let pc = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
            (pc, npc)
        };
        // Static mission entities follow the Original's 31 hidden pre-level
        // creations. For the following NPC (slot 1), frame 1 opens the
        // three-frame hearing cadence: (1 + 31 + 1) % 3 == 0.
        engine.control.frame_counter = 1;
        let Entity::Pc(pc_entity) = engine.get_entity_mut(pc).expect("noise PC exists") else {
            panic!("noise PC changed kind")
        };
        pc_entity.element.active = true;
        pc_entity.element.set_position_map(MapPoint::new(55.0, 0.0));
        pc_entity.pc.life_points = 100;
        let Entity::Soldier(npc_entity) = engine.get_entity_mut(npc).expect("listener exists")
        else {
            panic!("listener changed kind")
        };
        npc_entity.element.active = true;
        npc_entity.element.set_position_map(MapPoint::new(0.0, 0.0));
        npc_entity.npc.life_points = 100;
        npc_entity
            .npc
            .ai_brain
            .enemy_mut()
            .expect("listener has enemy AI")
            .base
            .me = npc.index();
        npc_entity.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(pc),
            detectable_type: DetectableType::Enemy,
            ..Detectable::default()
        });

        let mut movement = SequenceElement::new_movement(
            1,
            crate::element::Command::Move,
            Some(pc),
            OrderType::RunningUpright,
        );
        movement
            .orders
            .push_back(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        for (id, entity) in engine.world.entities.occupied() {
            positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
        }
        crate::sim_rng::with_seed(0xA013_0015, |sim| {
            engine.tick_actor_owner_envelopes(sim, &assets, &positions)
        });

        assert_eq!(
            engine
                .get_entity(pc)
                .and_then(Entity::actor_data)
                .expect("noise PC remains an actor")
                .last_noise_volume,
            70
        );
        engine
            .get_entity(npc)
            .and_then(Entity::npc_data)
            .expect("listener remains an NPC")
            .detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .find(|detectable| detectable.element == Some(pc))
            .expect("listener retains the PC detectable")
            .heard_last_frame
    }

    assert!(
        observe(true),
        "a PC must publish its current noise before a following NPC detects"
    );
    assert!(
        !observe(false),
        "an NPC before the PC must retain prior-frame noise for this slot"
    );
}

#[test]
fn pc_noise_refresh_invalidates_an_earlier_npc_tactical_snapshot() {
    use crate::element::{Camp, Detectable, DetectableType};

    let mut engine = EngineInner::new();
    let earlier_npc = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let pc = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let later_npc = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

    // The earlier NPC builds the lazy tactical snapshot before the PC's live
    // Human::Hourglass slot. The later NPC's cadence is open at frame zero:
    // (0 + 31 hidden creations + slot 2) % 3 == 0.
    engine.control.frame_counter = 0;
    for npc_id in [earlier_npc, later_npc] {
        let Entity::Soldier(npc) = engine.get_entity_mut(npc_id).expect("listener exists") else {
            panic!("listener changed kind")
        };
        npc.element.active = true;
        npc.element.set_position_map(MapPoint::new(0.0, 0.0));
        npc.npc.life_points = 100;
        npc.npc
            .ai_brain
            .enemy_mut()
            .expect("listener has enemy AI")
            .base
            .me = npc_id.index();
    }
    let Entity::Pc(pc_entity) = engine.get_entity_mut(pc).expect("noise PC exists") else {
        panic!("noise PC changed kind")
    };
    pc_entity.element.active = true;
    pc_entity.element.set_position_map(MapPoint::new(55.0, 0.0));
    pc_entity.pc.life_points = 100;
    pc_entity.actor.last_noise_volume = 200;
    pc_entity.actor.produced_noise = Some(crate::ai::Noise {
        origin: crate::ai::NoiseOrigin::from_position(crate::ai::Position {
            x: 55.0,
            y: 0.0,
            sector: None,
            level: 0,
        }),
        noise_type: crate::ai::NoiseType::TapTapTap,
        volume: 200,
        elevation: 0,
        element_id: u16::try_from(pc.index()).expect("test PC id fits noise record"),
    });
    pc_entity.actor.hear_noise_box =
        crate::coordinates::MapBBox::from_coords(-245.0, -220.0, 355.0, 220.0);

    let Entity::Soldier(later) = engine.get_entity_mut(later_npc).unwrap() else {
        unreachable!()
    };
    later.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(pc),
        detectable_type: DetectableType::Enemy,
        heard_last_frame: true,
        ..Detectable::default()
    });

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for (id, entity) in engine.world.entities.occupied() {
        positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
    }
    crate::sim_rng::with_seed(0xA013_0016, |sim| {
        engine.tick_actor_owner_envelopes(sim, &assets, &positions)
    });

    let actor = engine
        .get_entity(pc)
        .and_then(Entity::actor_data)
        .expect("noise PC remains an actor");
    assert_eq!(
        actor
            .produced_noise
            .expect("noise remains initialized")
            .volume,
        15
    );
    let later = engine
        .get_entity(later_npc)
        .and_then(Entity::npc_data)
        .expect("later listener remains an NPC");
    assert!(
        !later.detectable_lists[DetectableType::Enemy as usize][0].heard_last_frame,
        "later NPC must observe the PC's creation-ordered quiet refresh"
    );
}

#[test]
fn quiet_pc_noise_refresh_preserves_the_previous_hearing_box() {
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let pc = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let Entity::Pc(pc_entity) = engine.get_entity_mut(pc).expect("noise PC exists") else {
        panic!("noise PC changed kind")
    };
    pc_entity.element.active = true;
    pc_entity
        .element
        .set_position_map(MapPoint::new(100.0, 200.0));

    // An unclassified animation reaches RefreshProducedNoise's common tail:
    // volume zero and a +/-100 box around the current position.
    engine.refresh_pc_produced_noise_for_with_order(pc, OrderType::Invalid);
    let initial_box = engine
        .get_entity(pc)
        .and_then(Entity::actor_data)
        .expect("noise PC remains an actor")
        .hear_noise_box;
    assert_eq!(
        initial_box,
        crate::coordinates::MapBBox::from_coords(0.0, 100.0, 200.0, 300.0)
    );

    // Original's breath arm updates the noise position and volume, then
    // returns before rebuilding mboxHearMyNoiseBox. Preserve both halves of
    // that deliberately inconsistent state after the PC moves.
    let Entity::Pc(pc_entity) = engine.get_entity_mut(pc).unwrap() else {
        unreachable!()
    };
    pc_entity
        .element
        .set_position_map(MapPoint::new(210.0, 220.0));
    engine.refresh_pc_produced_noise_for_with_order(pc, OrderType::WaitingUpright);

    let actor = engine
        .get_entity(pc)
        .and_then(Entity::actor_data)
        .expect("noise PC remains an actor");
    let noise = actor
        .produced_noise
        .expect("quiet refresh still publishes the current noise record");
    assert_eq!(noise.volume, 15);
    assert_eq!((noise.origin.x, noise.origin.y), (210.0, 220.0));
    assert_eq!(actor.hear_noise_box, initial_box);
    assert!(
        !actor
            .hear_noise_box
            .contains_point(MapPoint::new(210.0, 220.0))
    );
}

#[test]
fn fused_owner_gates_keep_fried_frozen_and_inactive_original_boundaries() {
    use super::tick::{ActorOwnerEnvelopePhase as Phase, capture_actor_owner_envelope};

    let mut engine = EngineInner::new();
    let inactive = engine.add_entity(make_test_pc(crate::element::Posture::Dead));
    let fried = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let npc = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let Entity::Pc(inactive_pc) = engine.get_entity_mut(inactive).expect("inactive PC exists")
    else {
        panic!("inactive PC changed kind")
    };
    inactive_pc.element.active = false;
    inactive_pc
        .element
        .set_position_map(MapPoint::new(77.0, 88.0));
    inactive_pc.human.tiredness = 100;
    let Entity::Pc(fried_pc) = engine.get_entity_mut(fried).expect("fried PC exists") else {
        panic!("fried PC changed kind")
    };
    fried_pc.pc.fried_psykokwack = true;
    fried_pc.actor.produced_noise = None;
    engine.set_actors_frozen(true);
    engine.control.frame_counter = engine.world.original_creation_order(inactive) & 31;
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("inactive PC fixture has a character profile")
        .endurance = 100;
    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for (id, entity) in engine.world.entities.occupied() {
        positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
    }

    let (_, trace) = capture_actor_owner_envelope(|| {
        crate::sim_rng::with_seed(0xA013_6A7E, |sim| {
            engine.tick_actor_owner_envelopes(sim, &assets, &positions)
        })
    });

    assert!(
        !trace.iter().any(|phase| match phase {
            Phase::SoldierPrelude(owner)
            | Phase::Patrol(owner)
            | Phase::HumanPrelude(owner)
            | Phase::BaseActor(owner)
            | Phase::MovementExecute(owner)
            | Phase::HumanNoise(owner)
            | Phase::HumanTiredness(owner)
            | Phase::PcTail(owner)
            | Phase::NpcTail(owner) => *owner == fried,
        }),
        "fried PCs return before Human, Actor, noise, tiredness, and healing"
    );
    assert!(trace.contains(&Phase::BaseActor(npc)));
    assert!(trace.contains(&Phase::NpcTail(npc)));
    assert!(!trace.contains(&Phase::Patrol(npc)));
    let inactive_pc = engine
        .get_entity(inactive)
        .and_then(Entity::as_pc)
        .expect("inactive PC remains installed");
    let noise = inactive_pc
        .actor
        .produced_noise
        .expect("inactive PC still refreshes noise metadata");
    assert_eq!((noise.origin.x, noise.origin.y), (77.0, 88.0));
    assert_eq!(noise.volume, 0, "only inactivity/building zeroes PC noise");
    assert!(
        inactive_pc.human.tiredness < 100,
        "inactive/dead humans still recover tiredness on their staggered slot"
    );
    assert!(
        engine
            .get_entity(fried)
            .and_then(Entity::actor_data)
            .expect("fried PC remains an actor")
            .produced_noise
            .is_none(),
        "fried return must precede produced-noise refresh"
    );
}

#[test]
fn tiredness_recovery_uses_original_creation_order_cadence() {
    let mut engine = EngineInner::new();
    let restored = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let aligned = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
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
fn patrol_member_thinks_before_the_chief_applies_its_direction() {
    use crate::ai::{AiState, PathHistoryEntry, PathId, PatrolPath, Position, Substate};

    let mut engine = EngineInner::new();
    let chief = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let member = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    for id in [chief, member] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).expect("patrol soldier exists")
        else {
            panic!("patrol soldier changed kind")
        };
        soldier.element.active = true;
        soldier.npc.life_points = 100;
        soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("patrol soldier has enemy AI")
            .base
            .me = id.index();
    }
    let chief_position = Position {
        x: 100.0,
        y: 0.0,
        ..Position::default()
    };
    let Entity::Soldier(chief_entity) = engine.get_entity_mut(chief).unwrap() else {
        unreachable!()
    };
    chief_entity
        .element
        .set_position_map(MapPoint::new(chief_position.x, chief_position.y));
    chief_entity.element.sprite.position_iface.set_move_box(
        crate::coordinates::MoveBox::from_corners(
            crate::coordinates::MapVec::new(-2.0, -2.0),
            crate::coordinates::MapVec::new(2.0, 2.0),
        ),
    );
    let chief_ai = chief_entity.npc.ai_brain.base_mut().unwrap();
    chief_ai.current_state = AiState::Default;
    chief_ai.theoretical_patrol = vec![member];
    chief_ai.patrol = vec![member];
    chief_ai.patrol_path = Some(PatrolPath {
        hiking_path_index: PathId::new(0).unwrap(),
        current_waypoint_index: 0,
        last_waypoint_index: 0,
        forward: true,
        size: 1,
        history: vec![
            PathHistoryEntry {
                position: Position::default(),
                direction: 9,
                distance: 0,
            },
            PathHistoryEntry {
                position: chief_position,
                direction: 9,
                distance: 100,
            },
        ],
    });
    let Entity::Soldier(member_entity) = engine.get_entity_mut(member).unwrap() else {
        unreachable!()
    };
    member_entity
        .element
        .set_position_map(MapPoint::new(500.0, 0.0));
    member_entity.element.set_direction_instantly(3);
    let member_ai = member_entity.npc.ai_brain.base_mut().unwrap();
    member_ai.patrol_chief = Some(chief);
    member_ai.current_state = AiState::Default;
    member_ai.current_substate = Substate::DefaultPatrolEnrouteWaiting;
    engine.control.frame_counter = 0;
    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for (id, entity) in engine.world.entities.occupied() {
        positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
    }

    crate::sim_rng::with_seed(0xA013_7A70, |sim| {
        engine.tick_patrol_coordination_for_npc(sim, &assets, chief, &positions)
    });

    let member_entity = engine.get_entity(member).unwrap();
    let member_ai = member_entity.ai_controller().unwrap();
    assert_ne!(
        member_ai.current_substate,
        Substate::DefaultPatrolEnrouteWaiting,
        "patrol coordinate Think must leave waiting before direction is applied"
    );
    assert_eq!(member_ai.patrol_direction, 9);
    assert_eq!(
        member_entity.element_data().direction(),
        3,
        "CALL_PATROL_COORDINATE must leave the waiting substate before GetInstructedPatrolDirection; applying direction first would emit a face action"
    );
}

#[test]
fn patrol_direction_macro_effect_closes_at_the_chief_owner_boundary() {
    use crate::ai::Substate;
    use crate::element::{ActionState, Camp, Entity};

    let mut engine = EngineInner::new();
    let chief = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let member = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
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
    // FaceTo launches a sequence; the Turn element becomes the actor's live
    // order only when the sequence manager promotes it, so the synchronous
    // observable at the chief macro boundary is the about-to-be-launched
    // Turn, not an already-current Turning order.
    assert!(
        engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(member, crate::element::Command::Turn),
        "CMD_PATROL_DIRECTION must synchronously enqueue the waiting member's FaceTo Turn before the chief macro boundary returns"
    );
}

#[test]
fn patrol_refresh_uses_owner_relative_member_positions_and_spawn_fallback() {
    use crate::ai::AiState;
    use crate::element::{Camp, Entity};

    fn member_is_admitted(member_before_chief: bool, spawn_after_snapshot: bool) -> bool {
        let mut engine = EngineInner::new();
        let (chief, initial_member) = if member_before_chief {
            let member = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
            let chief = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
            (chief, Some(member))
        } else {
            let chief = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
            let member = (!spawn_after_snapshot)
                .then(|| engine.add_entity(make_test_ai_soldier(Camp::Lacklandists)));
            (chief, member)
        };
        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        for (id, entity) in engine.world.entities.occupied() {
            positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
        }
        let member = initial_member
            .unwrap_or_else(|| engine.add_entity(make_test_ai_soldier(Camp::Lacklandists)));

        for id in [chief, member] {
            let Entity::Soldier(soldier) = engine.get_entity_mut(id).unwrap() else {
                unreachable!()
            };
            soldier.element.active = true;
            soldier.npc.life_points = 100;
            soldier.npc.ai_brain.base_mut().unwrap().me = id.index();
            soldier.npc.ai_brain.base_mut().unwrap().current_state = AiState::Default;
        }
        engine
            .get_entity_mut(chief)
            .unwrap()
            .element_data_mut()
            .set_position_map(MapPoint::new(0.0, 0.0));
        let member_before = MapPoint::new(50.0, 0.0);
        let member_after = if member_before_chief {
            MapPoint::new(500.0, 0.0)
        } else {
            MapPoint::new(50.0, 0.0)
        };
        if positions.get(member).is_some() {
            positions[member] = Some(crate::entities::BoundaryPosition {
                map: member_before,
                world: crate::coordinates::WorldPoint3D::new(member_before.x, member_before.y, 0.0),
            });
        }
        engine
            .get_entity_mut(member)
            .unwrap()
            .element_data_mut()
            .set_position_map(member_after);
        let Entity::Soldier(chief_entity) = engine.get_entity_mut(chief).unwrap() else {
            unreachable!()
        };
        chief_entity.npc.view_radius = 100;
        let chief_ai = chief_entity.npc.ai_brain.base_mut().unwrap();
        chief_ai.needs_patrol_reinit = true;
        chief_ai.theoretical_patrol = vec![member];

        crate::sim_rng::with_seed(0x0A01_3705, |sim| {
            engine.tick_patrol_coordination_for_npc(sim, &assets, chief, &positions)
        });
        engine
            .get_entity(chief)
            .unwrap()
            .ai_controller()
            .unwrap()
            .patrol
            .contains(&member)
    }

    assert!(
        member_is_admitted(false, false),
        "an earlier chief must see a later member at its preserved pre-movement position"
    );
    assert!(
        !member_is_admitted(true, false),
        "a later chief must see an earlier member at its already-completed post-movement position"
    );
    assert!(
        member_is_admitted(false, true),
        "a callback-spawned later member absent from the oracle must use its current, never-moved position"
    );
}

#[test]
fn inactive_dead_patrol_chief_still_records_eligible_history() {
    use crate::ai::{AiState, PathId, PatrolPath};
    use crate::element::{Camp, Entity};

    let mut engine = EngineInner::new();
    let chief = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let member = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let Entity::Soldier(chief_entity) = engine.get_entity_mut(chief).unwrap() else {
        unreachable!()
    };
    chief_entity.element.active = false;
    chief_entity.npc.life_points = 0;
    let chief_ai = chief_entity.npc.ai_brain.base_mut().unwrap();
    chief_ai.current_state = AiState::Default;
    chief_ai.patrol = vec![member];
    chief_ai.patrol_path = Some(PatrolPath {
        hiking_path_index: PathId::new(0).unwrap(),
        current_waypoint_index: 0,
        last_waypoint_index: 0,
        forward: true,
        size: 1,
        history: Vec::new(),
    });
    engine.control.frame_counter = 1;
    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for (id, entity) in engine.world.entities.occupied() {
        positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
    }

    crate::sim_rng::with_seed(0xA013_DEAD, |sim| {
        engine.tick_patrol_coordination_for_npc(sim, &assets, chief, &positions)
    });

    assert_eq!(
        engine
            .get_entity(chief)
            .unwrap()
            .ai_controller()
            .unwrap()
            .patrol_path
            .as_ref()
            .unwrap()
            .history
            .len(),
        1,
        "RefreshPatrol has no non-Original active/dead chief gate before its per-frame history write"
    );
}

#[test]
#[should_panic(expected = "missing its required AI controller while applying recovery state")]
fn think_with_drain_rejects_a_soldier_missing_its_required_ai() {
    use crate::ai::{AiContext, AiPerTickData, Stimulus, StimulusType};

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let npc_id = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));

    engine.dispatch_think_with_drain(
        sim,
        npc_id,
        &Stimulus::new(StimulusType::EventDone),
        &AiContext::default(),
        &AiPerTickData::stub(),
        &LevelAssets::new(),
    );
}

#[test]
fn npc_post_detection_tail_is_wholly_creation_ordered_even_without_detection() {
    use super::ai::{NpcPostDetectionTailPhase as Tail, capture_npc_post_detection_tail_phases};

    let mut engine = EngineInner::new();
    let first = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let second = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    for id in [first, second] {
        let entity = engine.get_entity_mut(id).expect("tail owner exists");
        entity.element_data_mut().active = true;
        let npc = entity.npc_data_mut().expect("tail owner has NPC data");
        for list in &mut npc.detectable_lists {
            list.clear();
        }
        let ai = npc.ai_brain.base_mut().expect("tail owner has AI");
        ai.timer_is_running = false;
        ai.macro_timer_is_running = false;
    }

    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for (id, entity) in engine.world.entities.occupied() {
        positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
    }
    let (_, trace) = capture_npc_post_detection_tail_phases(|| {
        crate::sim_rng::with_seed(0xA013_7A11, |sim| {
            engine.tick_enemy_ai_with_creation_ordered_prelude(sim, &assets, &positions)
        })
    });

    let whole_tail = [
        Tail::Ambush,
        Tail::Deafness,
        Tail::Busy,
        Tail::Ladder,
        Tail::RandomSpeech,
        Tail::LockGate,
        Tail::SixteenthFrame,
        Tail::NormalTimer,
        Tail::MacroTimer,
        Tail::Emoticon,
        Tail::QueuedStimuli,
    ];
    let expected: Vec<_> = [first, second]
        .into_iter()
        .flat_map(|id| whole_tail.into_iter().map(move |phase| (id, phase)))
        .collect();
    assert_eq!(trace, expected);
}

#[test]
fn locked_owner_stops_at_gate_without_blocking_later_unlocked_owner() {
    use super::ai::{NpcPostDetectionTailPhase as Tail, capture_npc_post_detection_tail_phases};
    use crate::ai::AiLockFlags;

    let mut engine = EngineInner::new();
    let locked = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let unlocked = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    for id in [locked, unlocked] {
        let entity = engine.get_entity_mut(id).expect("gate owner exists");
        entity.element_data_mut().active = true;
        for list in &mut entity
            .npc_data_mut()
            .expect("gate owner has NPC data")
            .detectable_lists
        {
            list.clear();
        }
    }
    let ai = engine
        .get_entity_mut(locked)
        .and_then(Entity::ai_controller_mut)
        .expect("locked owner has AI");
    ai.locks_flag_field = AiLockFlags::FREEZE;
    ai.when_does_timer_ring = u32::MAX;
    ai.when_does_macro_timer_ring = u32::MAX;
    ai.emoticon_expiration_date = u32::MAX;

    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for (id, entity) in engine.world.entities.occupied() {
        positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
    }
    let (_, trace) = capture_npc_post_detection_tail_phases(|| {
        crate::sim_rng::with_seed(0xA013_10CC, |sim| {
            engine.tick_enemy_ai_with_creation_ordered_prelude(sim, &assets, &positions)
        })
    });
    let locked_trace: Vec<_> = trace
        .iter()
        .filter_map(|(id, phase)| (*id == locked).then_some(*phase))
        .collect();
    assert_eq!(
        locked_trace,
        vec![
            Tail::Ambush,
            Tail::Deafness,
            Tail::Busy,
            Tail::Ladder,
            Tail::RandomSpeech,
            Tail::LockGate,
        ]
    );
    assert_eq!(
        trace.last(),
        Some(&(unlocked, Tail::QueuedStimuli)),
        "the later owner must execute its whole unlocked tail"
    );
    let ai = engine
        .get_entity(locked)
        .and_then(Entity::ai_controller)
        .expect("locked owner retains AI");
    assert_eq!(ai.when_does_timer_ring, 0);
    assert_eq!(ai.when_does_macro_timer_ring, 0);
    assert_eq!(ai.emoticon_expiration_date, 0);
}

#[test]
fn post_detection_tail_clears_only_unlocked_expired_emoticon() {
    use crate::ai::{AiLockFlags, EmoticonType};

    let mut engine = EngineInner::new();
    let unlocked = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let locked = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.control.frame_counter = 200;
    for id in [unlocked, locked] {
        let ai = engine
            .get_entity_mut(id)
            .and_then(Entity::ai_controller_mut)
            .expect("emoticon owner has AI");
        ai.set_transient_emoticon(EmoticonType::QuestionMark, 1, 100);
    }
    engine
        .get_entity_mut(locked)
        .and_then(Entity::ai_controller_mut)
        .expect("locked emoticon owner has AI")
        .locks_flag_field = AiLockFlags::FREEZE;

    crate::sim_rng::with_seed(0xA013_EA10, |sim| {
        engine.tick_npc_post_detection_tail_for_npc(sim, unlocked, &assets);
        engine.tick_npc_post_detection_tail_for_npc(sim, locked, &assets);
    });
    let unlocked_ai = engine
        .get_entity(unlocked)
        .and_then(Entity::ai_controller)
        .expect("unlocked emoticon owner retains AI");
    assert_eq!(unlocked_ai.current_emoticon_type, EmoticonType::None);
    assert!(!unlocked_ai.emoticon_has_expiration_date);
    let locked_ai = engine
        .get_entity(locked)
        .and_then(Entity::ai_controller)
        .expect("locked emoticon owner retains AI");
    assert_eq!(locked_ai.current_emoticon_type, EmoticonType::QuestionMark);
    assert!(locked_ai.emoticon_has_expiration_date);
    assert_eq!(locked_ai.emoticon_expiration_date, 102);
}

#[test]
fn post_detection_tail_refreshes_deafness_off_acoustic_cadence() {
    let mut engine = EngineInner::new();
    let npc_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.control.frame_counter = 7;
    assert_ne!((engine.control.frame_counter + npc_id.index()) % 3, 0);
    let npc = engine
        .get_entity_mut(npc_id)
        .and_then(Entity::npc_data_mut)
        .expect("deafness owner has NPC data");
    npc.old_cover_noise_deafness = 100;
    npc.old_cover_noise_deafness_frame_counter = 6;

    crate::sim_rng::with_seed(0xA013_DEAF, |sim| {
        engine.tick_npc_post_detection_tail_for_npc(sim, npc_id, &assets)
    });
    let npc = engine
        .get_entity(npc_id)
        .and_then(Entity::npc_data)
        .expect("deafness owner retains NPC data");
    assert_eq!(npc.old_cover_noise_deafness_frame_counter, 7);
    assert!(npc.old_cover_noise_deafness < 100);
}

#[test]
fn ambush_refresh_drains_look_sidewards_before_next_tail_phase() {
    use crate::ai::{AiState, AmbushPoint, Position, Substate};
    use crate::ai_enemy::AmbushPointStatus;
    use crate::element::Command;

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let npc_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let Entity::Soldier(soldier) = engine.get_entity_mut(npc_id).expect("ambush owner exists")
    else {
        panic!("ambush owner changed kind")
    };
    soldier.element.active = true;
    soldier.element.set_position_map(MapPoint::new(0.0, 0.0));
    soldier.element.set_direction_instantly(0);
    let enemy = soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("ambush owner has enemy AI");
    enemy.base.current_state = AiState::Seeking;
    enemy.base.current_substate = Substate::SeekingSeekpoint;
    enemy.soldier_profile_iq = 100;
    enemy.ambush_point_status = vec![AmbushPointStatus::Near];
    engine.ai.global.ambush_points = vec![AmbushPoint {
        position: Position {
            x: 10.0,
            y: 0.0,
            ..Position::default()
        },
        direction: 0,
        position_3d: crate::coordinates::WorldPoint3D::new(10.0, 0.0, 32.0),
        id: 0,
    }];

    engine.tick_refresh_ambush_points_for_npc(sim, npc_id, &assets);

    let enemy = engine
        .get_entity(npc_id)
        .and_then(Entity::enemy_ai)
        .expect("ambush owner retains enemy AI");
    assert_eq!(
        enemy.base.current_substate,
        Substate::SeekingSeekpointCheckingAmbushPoint
    );
    assert!(enemy.base.outbox.actor.look_sidewards.is_none());
    assert!(
        [Command::LookLeft, Command::LookRight]
            .into_iter()
            .any(|command| engine.actor_command(npc_id) == command
                || engine
                    .orders
                    .sequence_manager
                    .element_is_about_to_be_launched(npc_id, command)),
        "RefreshAmbushPoints must launch LookSidewards before deafness/busy"
    );
}

#[test]
fn post_detection_tail_preserves_ladder_threshold_and_macro_stop_semantics() {
    use crate::ai::Substate;
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let npc_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let mut wait = SequenceElement::new_generic(1, Command::Wait, Some(npc_id));
    wait.state = SequenceState::InProgress;
    engine.orders.sequence_manager.launch_element(wait);
    let Entity::Soldier(soldier) = engine.get_entity_mut(npc_id).expect("ladder owner exists")
    else {
        panic!("ladder owner changed kind")
    };
    soldier.element.posture = Posture::OnLadder;
    soldier.npc.stuck_on_ladder_emergency_counter = 25;
    let ai = soldier
        .npc
        .ai_brain
        .base_mut()
        .expect("ladder owner has AI");
    ai.macro_timer_is_running = true;
    ai.when_does_macro_timer_ring = 0;
    ai.current_substate = Substate::DefaultOnPost;

    engine.tick_npc_stuck_on_ladder_for_npc(sim, npc_id, &assets);
    assert_eq!(
        engine
            .get_entity(npc_id)
            .and_then(Entity::npc_data)
            .expect("ladder owner retains NPC data")
            .stuck_on_ladder_emergency_counter,
        0,
        "the 26th qualifying frame must trigger recovery and reset"
    );

    engine.tick_ai_macro_timer_for_npc(sim, npc_id, &assets);
    assert!(
        !engine
            .get_entity(npc_id)
            .and_then(Entity::ai_controller)
            .expect("macro owner retains AI")
            .macro_timer_is_running,
        "elapsed macro timers stop even outside DefaultInMacro"
    );
}

#[test]
fn normal_timer_uses_unsigned_wrapped_overflow_guard() {
    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let npc_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.control.frame_counter = u32::MAX - 10;
    let ai = engine
        .get_entity_mut(npc_id)
        .and_then(Entity::ai_controller_mut)
        .expect("overflow-timer owner has AI");
    ai.timer_is_running = true;
    ai.when_does_timer_ring = u32::MAX - 5;
    ai.substate_at_last_timer_launch = ai.current_substate;

    engine.tick_ai_normal_timer_for_npc(sim, npc_id, &assets);
    let ai = engine
        .get_entity(npc_id)
        .and_then(Entity::ai_controller)
        .expect("overflow-timer owner retains AI");
    assert!(
        !ai.timer_is_running || ai.when_does_timer_ring != u32::MAX - 5,
        "the wrapped million-frame guard must consume the apparently-future timer"
    );
}

#[test]
fn normal_timer_does_not_turn_alerted_soldier_toward_primary_target() {
    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let npc_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine
        .get_entity_mut(target)
        .unwrap()
        .element_data_mut()
        .set_position_map(MapPoint::new(0.0, 100.0));
    let Entity::Soldier(soldier) = engine.get_entity_mut(npc_id).unwrap() else {
        panic!("timer owner changed kind")
    };
    soldier.element.set_position_map(MapPoint::ZERO);
    soldier.element.set_direction_instantly(5);
    soldier.npc.alerted = true;
    let ai = soldier.npc.ai_brain.base_mut().unwrap();
    ai.primary_target = Some(crate::ai::AiEntityHandle::new(target.index()));
    ai.script_locked = true;
    ai.timer_is_running = true;
    ai.when_does_timer_ring = 0;
    ai.substate_at_last_timer_launch = ai.current_substate;

    engine.tick_ai_normal_timer_for_npc(sim, npc_id, &assets);

    let element = engine.get_entity(npc_id).unwrap().element_data();
    assert_eq!(element.direction(), 5);
    assert_eq!(
        element.sprite.position_iface.get_direction_goal().as_u8(),
        5,
        "RHElementActorNPC::Focus changes only the view cone; normal timer dispatch does not turn the actor"
    );
}

#[test]
fn retained_fifo_stops_when_first_think_acquires_busy_lock() {
    use crate::ai::{AiLockFlags, Stimulus, StimulusType};
    use crate::element::Posture;

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let npc_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let Entity::Soldier(soldier) = engine.get_entity_mut(npc_id).expect("FIFO owner exists") else {
        panic!("FIFO owner changed kind")
    };
    soldier.element.posture = Posture::OnLadder;
    let ai = soldier.npc.ai_brain.base_mut().expect("FIFO owner has AI");
    ai.locks_flag_field = AiLockFlags::empty();
    ai.stimulus_queue = vec![
        Stimulus::new(StimulusType::EventAfterScriptGoOn),
        Stimulus::new(StimulusType::EventCouldntReachPoint),
        Stimulus::new(StimulusType::EventAfterScriptGoOn),
        Stimulus::new(StimulusType::EventTimer),
    ];

    engine.tick_ai_queued_stimuli_for_npc(sim, npc_id, &assets);
    let ai = engine
        .get_entity(npc_id)
        .and_then(Entity::ai_controller)
        .expect("FIFO owner retains AI");
    assert!(ai.locks_flag_field.contains(AiLockFlags::BUSY));
    assert_eq!(
        ai.stimulus_queue
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![
            StimulusType::EventAfterScriptGoOn,
            StimulusType::EventTimer,
            StimulusType::EventCouldntReachPoint,
        ],
        "the lock check must preserve both a later duplicate marker and its suffix before the causal retry"
    );
}

#[test]
fn panic_generated_reachpoint_precedes_retained_panic_sibling_and_draws_twice() {
    use crate::ai::{AiState, Position, Stimulus, StimulusType, Substate};
    use crate::sim_rng::{RngSite, with_draw_trace};

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let npc_id = engine.add_entity(make_test_civilian(crate::element::Posture::Upright));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .civilians
        .push(crate::profiles::CivilianProfile::default());

    // Keep the first panic segment inside an open grid.  There are no door
    // seek records, so the first retained EVENT_PANIC must enter the
    // Original no-door branch and recursively Think(EVENT_REACHPOINT).
    engine.world.fast_grid_mut().size_map(64, 64);
    engine.world.fast_grid_mut().allocate_layers(1);
    let sector = crate::position_interface::SectorHandle::new(1).unwrap();
    let Entity::Civilian(civilian) = engine.get_entity_mut(npc_id).unwrap() else {
        panic!("retained panic owner changed kind")
    };
    civilian.element.active = true;
    civilian
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(1000.0, 1000.0, 0.0));
    civilian.element.set_layer(0);
    civilian.element.set_sector(Some(sector));
    civilian
        .element
        .sprite
        .position_iface
        .set_move_box(crate::coordinates::MoveBox::from_corners(
            crate::coordinates::MapVec::new(-2.0, -2.0),
            crate::coordinates::MapVec::new(2.0, 2.0),
        ));
    civilian.npc.life_points = 100;
    let ai = civilian.npc.ai_brain.base_mut().unwrap();
    ai.current_state = AiState::Default;
    ai.current_substate = Substate::DefaultEnroute;
    ai.script_locked = false;
    ai.outbox
        .reentrant
        .self_stimuli
        .push(StimulusType::EventTimer.into());
    ai.stimulus_queue = vec![
        Stimulus::new(StimulusType::EventAfterScriptGoOn),
        Stimulus::with_position(
            StimulusType::EventPanic,
            Position {
                x: 900.0,
                y: 1000.0,
                sector: Some(sector),
                level: 0,
            },
        ),
        Stimulus::new(StimulusType::EventAfterScriptGoOn),
        Stimulus::with_position(
            StimulusType::EventPanic,
            Position {
                x: 1100.0,
                y: 1000.0,
                sector: Some(sector),
                level: 0,
            },
        ),
    ];

    let (_, draws) =
        with_draw_trace(|| engine.tick_ai_queued_stimuli_for_npc(sim, npc_id, &assets));

    assert_eq!(draws, vec![RngSite::AiPanic, RngSite::AiPanic]);
    let ai = engine
        .get_entity(npc_id)
        .and_then(Entity::ai_controller)
        .expect("retained panic owner keeps AI");
    let events = ai
        .ai_log
        .iter()
        .filter(|line| line.line_type == crate::ai::LogLineType::Event)
        .map(|line| line.info)
        .collect::<Vec<_>>();
    assert_eq!(
        events,
        vec![
            StimulusType::EventAfterScriptGoOn as u16,
            StimulusType::EventPanic as u16,
            StimulusType::EventReachPoint as u16,
            StimulusType::EventTimer as u16,
            StimulusType::EventPanic as u16,
        ],
        "Panic's direct recursive Think must precede both an existing self backlog and the retained sibling"
    );
}

#[test]
fn sampled_open_gate_does_not_recheck_lock_or_global_freeze_inside_suffix() {
    use crate::ai::{AiLockFlags, EmoticonType, StimulusType, Substate};

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let npc_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.control.frame_counter = 100;

    assert!(
        !engine.tick_npc_lock_gate_for_npc(npc_id),
        "the pre-suffix gate is sampled open"
    );

    // Model a synchronous The16thFrame/EVENT_TIMER consequence that acquires
    // both kinds of outer lock after the branch has already been entered.
    engine.set_actors_frozen(true);
    let ai = engine
        .get_entity_mut(npc_id)
        .and_then(Entity::ai_controller_mut)
        .expect("post-gate owner has AI");
    ai.locks_flag_field = AiLockFlags::FREEZE;
    ai.timer_is_running = true;
    ai.when_does_timer_ring = 100;
    ai.macro_timer_is_running = true;
    ai.when_does_macro_timer_ring = 100;
    ai.current_substate = Substate::DefaultOnPost;
    ai.set_transient_emoticon(EmoticonType::QuestionMark, 1, 99);

    engine.tick_ai_normal_timer_for_npc(sim, npc_id, &assets);
    engine.tick_ai_macro_timer_for_npc(sim, npc_id, &assets);
    engine.tick_npc_emoticon_expiration_for_npc(npc_id);
    engine.tick_ai_queued_stimuli_for_npc(sim, npc_id, &assets);

    let ai = engine
        .get_entity(npc_id)
        .and_then(Entity::ai_controller)
        .expect("post-gate owner retains AI");
    assert!(!ai.timer_is_running, "due normal timer is consumed");
    assert!(!ai.macro_timer_is_running, "due macro timer is consumed");
    assert_eq!(ai.current_emoticon_type, EmoticonType::None);
    assert!(
        ai.stimulus_queue
            .iter()
            .any(|stimulus| stimulus.stimulus_type == StimulusType::EventTimer),
        "Think observes the new AI lock and the retained loop preserves it"
    );
}

#[test]
fn civilian_timer_retained_self_and_macro_boundaries_launch_orders_immediately() {
    use crate::ai::{AiState, Position, Stimulus, StimulusType, Substate};
    use crate::element::Command;

    fn add_ready_civilian(engine: &mut EngineInner) -> EntityId {
        let id = engine.add_entity(make_test_civilian(crate::element::Posture::Upright));
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
    let target = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
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
    // The16thFrame's cadence is keyed on the NPC's register number (0 for
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
        "The16thFrame must run and hand its retried GoTo to the engine boundary"
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
fn civilian_macro_break_drains_missed_friend_detectables_immediately() {
    use crate::ai::{AiState, Substate};
    use crate::element::{Detectable, DetectableType};

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let civilian_id = engine.add_entity(make_test_civilian(crate::element::Posture::Upright));
    let friend_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Civilian(civilian) = engine
        .get_entity_mut(civilian_id)
        .expect("macro civilian exists")
    else {
        panic!("macro owner changed kind")
    };
    civilian.element.active = true;
    civilian.npc.life_points = 100;
    civilian.npc.detectable_lists[DetectableType::MissedFriend as usize].push(Detectable {
        element: Some(friend_id),
        detectable_type: DetectableType::MissedFriend,
        ..Detectable::default()
    });
    let ai = civilian
        .npc
        .ai_brain
        .base_mut()
        .expect("macro civilian has AI");
    ai.current_state = AiState::Default;
    ai.current_substate = Substate::DefaultInMacro;
    // CMD_FACE_TO without its u16 operand takes the common BreakMacro path,
    // which queues the MissedFriend deletions through set_checkpoint_charly(0).
    ai.macro_command = vec![3];
    ai.macro_command_offset = 0;
    ai.number_of_remaining_macro_bytes = 1;
    ai.macro_in_progress = true;
    ai.macro_timer_is_running = true;
    ai.when_does_macro_timer_ring = 0;

    engine.tick_ai_macro_timer_for_npc(sim, civilian_id, &assets);

    let civilian = engine
        .get_entity(civilian_id)
        .and_then(Entity::npc_data)
        .expect("macro civilian retains NPC data");
    assert!(
        civilian.detectable_lists[DetectableType::MissedFriend as usize].is_empty(),
        "common macro completion deletes must be applied to civilian NpcData"
    );
    let ai = engine
        .get_entity(civilian_id)
        .and_then(Entity::ai_controller)
        .expect("macro civilian retains AI");
    assert!(ai.outbox.actor.delete_detectables.is_empty());
}

#[test]
fn owner_tail_and_empty_common_drain_do_not_draw_unrelated_building_exit_gate() {
    use crate::ai::AmbushPoint;
    use crate::element::ActiveDoorPass;
    use crate::fast_find_grid::GridSector;
    use crate::gate::{Door, DoorIndex, DoorType};
    use crate::scb::{ClassEntry, SCB_VERSION, ScbFile};
    use crate::sector::{SectorNumber, SectorType};
    use crate::sim_rng::{RngSite, with_draw_trace};
    use std::collections::VecDeque;

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let quiet_owner = engine.add_entity(make_test_civilian(crate::element::Posture::Upright));
    let door_actor = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Civilian(civilian) = engine
        .get_entity_mut(quiet_owner)
        .expect("quiet civilian exists")
    else {
        panic!("quiet owner changed kind")
    };
    civilian.element.active = true;
    civilian.npc.life_points = 100;
    let ai = civilian
        .npc
        .ai_brain
        .base_mut()
        .expect("quiet civilian has AI");
    ai.current_state = crate::ai::AiState::Default;
    ai.current_substate = crate::ai::Substate::DefaultInMacro;
    ai.timer_is_running = false;
    ai.macro_command = vec![3, 8, 0]; // CMD_FACE_TO(8)
    ai.macro_command_offset = 0;
    ai.number_of_remaining_macro_bytes = 3;
    ai.macro_timer_is_running = true;
    ai.when_does_macro_timer_ring = 0;
    ai.stimulus_queue.push(crate::ai::Stimulus::new(
        crate::ai::StimulusType::EventOutOfView,
    ));

    let Entity::Pc(pc) = engine
        .get_entity_mut(door_actor)
        .expect("door-passing actor exists")
    else {
        panic!("door-passing actor changed kind")
    };
    pc.element.active = true;
    pc.pc.life_points = 100;
    pc.actor.active_door_pass = Some(ActiveDoorPass {
        door_index: DoorIndex::new(0).expect("valid door index"),
        direct: true,
        position_direct: true,
        steps: VecDeque::new(),
        triggers_fired: 0,
        current_action: crate::order::OrderType::default(),
        current_reverse: false,
        saved_action_state: None,
    });
    pc.actor.passing_door_directly = true;
    // Forecast preparation only treats the actor as mid door transit while
    // its position interface still holds the live door pointer.
    pc.element.sprite.position_iface.set_door_for_test(
        crate::position_interface::DoorHandle::new(0).expect("valid door index"),
    );

    // Original `RHActor::IsPassingDoor` observes the selected PassDoor
    // sequence command. Runtime door mirrors alone no longer arm forecast
    // preparation after the selected-command parity fix.
    let mut pass = crate::sequence::SequenceElement::new_movement(
        1,
        crate::element::Command::PassDoor,
        Some(door_actor),
        crate::order::OrderType::WalkingUpright,
    );
    if let crate::sequence::SequenceElementData::Movement {
        gate_id, direction, ..
    } = &mut pass.data
    {
        *gate_id = Some(DoorIndex::new(0).expect("valid door index"));
        *direction = 1;
    } else {
        unreachable!("PassDoor fixture must be a movement element")
    }
    let pass_sequence = engine.orders.sequence_manager.launch_element(pass);
    engine
        .orders
        .sequence_manager
        .element_in_progress(pass_sequence, 0);

    let building_sector = SectorNumber::new(8);
    engine.script_domains.interactables.doors = vec![
        Door {
            door_type: DoorType::Building,
            sector_out: SectorNumber::new(7),
            sector_in: building_sector,
            sector_out_index: crate::fast_find_grid::SectorIndex::new(1),
            sector_in_index: crate::fast_find_grid::SectorIndex::new(0),
            point_out: MapPoint::new(0.0, 0.0),
            point_in: MapPoint::new(10.0, 0.0),
            ..Door::default()
        },
        Door {
            door_type: DoorType::Building,
            sector_out: SectorNumber::new(9),
            sector_in: building_sector,
            sector_out_index: crate::fast_find_grid::SectorIndex::new(2),
            sector_in_index: crate::fast_find_grid::SectorIndex::new(0),
            point_out: MapPoint::new(100.0, 0.0),
            point_in: MapPoint::new(90.0, 0.0),
            ..Door::default()
        },
    ];
    let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
    for (index, sector_number) in [building_sector, SectorNumber::new(7), SectorNumber::new(9)]
        .into_iter()
        .enumerate()
    {
        level.sector_number_map.insert(sector_number, index);
        level.sectors.push(GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: if index == 0 {
                SectorType::BUILDING
            } else {
                SectorType::MOTION | SectorType::AREA
            },
            layer: 0,
            sector_number,
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
    engine.scripts.mission = Some(
        MissionScript::from_scb(ScbFile {
            version: SCB_VERSION,
            classes: vec![ClassEntry {
                source_file: "pa013_rng_test.scs".into(),
                class_name: "StartUp".into(),
                size_of_member_variables: 0,
                member_variables: Vec::new(),
                functions: Vec::new(),
                quads: Vec::new(),
            }],
        })
        .expect("minimal mission script builds"),
    );
    engine.ai.global.ambush_points = vec![AmbushPoint {
        position: crate::ai::Position::default(),
        direction: 0,
        position_3d: crate::coordinates::WorldPoint3D::default(),
        id: 0,
    }];
    engine.control.frame_counter = (quiet_owner.index() + 101) & 255;

    // Building scratch no longer burns BuildingExitGate eagerly: the
    // prepared forecast defers the gate selection draw until an AI statement
    // actually resolves it. Prove the fixture is armed by resolving the
    // door-passing actor's prepared forecast directly.
    let control_scratch = engine.build_sim_scratch(sim, &assets);
    let (_, control_trace) = with_draw_trace(|| {
        control_scratch
            .ai_entity_views
            .get(&door_actor.index())
            .expect("door-passing actor has an AI entity view")
            .forecasted_destination
            .resolve(sim);
    });
    drop(control_scratch);
    assert!(
        control_trace.contains(&RngSite::BuildingExitGate),
        "the fixture must exercise BuildingExitGate when its prepared forecast is resolved"
    );

    let (_, empty_drain_trace) =
        with_draw_trace(|| engine.drain_pending_for_npc(sim, quiet_owner, &assets));
    assert!(
        !empty_drain_trace.contains(&RngSite::BuildingExitGate),
        "an empty common outbox drain must not build forecast scratch"
    );

    let (_, tail_trace) =
        with_draw_trace(|| engine.tick_npc_post_detection_tail_for_npc(sim, quiet_owner, &assets));
    assert!(
        !tail_trace.contains(&RngSite::BuildingExitGate),
        "due macro and retained Think work must not forecast an unrelated door-passing actor"
    );
}

#[test]
fn npc_body_broadcast_respects_swapped_creation_order_boundary() {
    use crate::ai::{AiLockFlags, AiState, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, DetectableType, ElementData, ElementKind, Entity, Posture};

    #[derive(Debug, PartialEq)]
    struct Observation {
        body_before_observer: bool,
        body_slot: u32,
        observer_slot: u32,
        retained_stimuli: Vec<StimulusType>,
        body_detectables_after_tick: Vec<u32>,
        inform_flag_after_tick: bool,
    }

    fn observe(body_before_observer: bool) -> Observation {
        let mut engine = EngineInner::new();
        // Keep both NPCs away from the slot-zero special value used by a few
        // legacy AI handles, without introducing another detectable human.
        engine.add_entity(Entity::Target(crate::element::ElementTarget {
            element: ElementData {
                kind: ElementKind::Target,
                ..ElementData::default()
            },
            fx: Default::default(),
            target: Default::default(),
        }));

        let body = make_test_ai_soldier(Camp::Lacklandists);
        let observer = make_test_ai_soldier(Camp::Lacklandists);
        let (body_id, observer_id) = if body_before_observer {
            (engine.add_entity(body), engine.add_entity(observer))
        } else {
            let observer_id = engine.add_entity(observer);
            let body_id = engine.add_entity(body);
            (body_id, observer_id)
        };

        for (id, x) in [(observer_id, 0.0), (body_id, 40.0)] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("creation-order body test soldier exists")
            else {
                panic!("creation-order body test entity changed kind")
            };
            soldier.element.active = true;
            soldier
                .element
                .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
            soldier.element.set_position_map(MapPoint::new(x, 0.0));
            soldier.element.set_direction_instantly(4);
            soldier.npc.direction_old = 4;
            soldier.npc.life_points = 100;
            soldier.npc.view_radius = 135;
            soldier.npc.eye_status = crate::element::EyeStatus::Stare;
            let ai = soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("creation-order body test soldier has enemy AI");
            ai.base.me = id.index();
        }

        let Entity::Soldier(body) = engine
            .get_entity_mut(body_id)
            .expect("body exists before fixture completion")
        else {
            panic!("body changed kind")
        };
        body.human.unconscious = true;
        body.element.posture = Posture::Lying;
        body.npc.inform_my_friends = true;

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);

        // Isolate BODY detection and retain its raw stimulus so handler state
        // changes do not obscure whether this creation slot actually saw it.
        for id in [body_id, observer_id] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("body boundary soldier survives fixture completion")
            else {
                panic!("body boundary soldier changed kind after fixture")
            };
            for list in &mut soldier.npc.detectable_lists {
                list.clear();
            }
        }
        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("body observer survives fixture completion")
        else {
            panic!("body observer changed kind after fixture")
        };
        let ai = observer
            .npc
            .ai_brain
            .enemy_mut()
            .expect("body observer retains enemy AI");
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingJustWatching;
        ai.current_task_priority = task_priority::NONE;
        ai.base.locks_flag_field = AiLockFlags::FREEZE;

        // The Body bucket refreshes strictly on the modulo-8 cadence of the
        // observer's modified frame (universal frame + creation order); open
        // that gate so the boundary question — did the broadcast land before
        // or after the observer's slot — is what decides the outcome.
        let observer_order = engine.world.original_creation_order(observer_id);
        engine.control.frame_counter = (8 - (observer_order % 8)) % 8;

        let mut positions_before_movement =
            crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        for (entity_id, entity) in engine.world.entities.occupied() {
            positions_before_movement[entity_id] =
                Some(crate::entities::BoundaryPosition::of(entity.element_data()));
        }

        crate::sim_rng::with_seed(0xA013_0B0D, |sim| {
            engine.tick_enemy_ai_with_creation_ordered_prelude(
                sim,
                &assets,
                &positions_before_movement,
            )
        });

        let observer = engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("body observer remains an NPC");
        let observer_ai = observer
            .ai_brain
            .enemy()
            .expect("body observer remains enemy AI");
        let body = engine
            .get_entity(body_id)
            .and_then(Entity::npc_data)
            .expect("body remains an NPC");

        Observation {
            body_before_observer,
            body_slot: body_id.index(),
            observer_slot: observer_id.index(),
            retained_stimuli: observer_ai
                .base
                .stimulus_queue
                .iter()
                .map(|stimulus| stimulus.stimulus_type)
                .collect(),
            body_detectables_after_tick: observer.detectable_lists[DetectableType::Body as usize]
                .iter()
                .map(|detectable| {
                    detectable
                        .element
                        .expect("broadcast BODY detectable must retain its source")
                        .index()
                })
                .collect(),
            inform_flag_after_tick: body.inform_my_friends,
        }
    }

    assert_eq!(
        observe(true),
        Observation {
            body_before_observer: true,
            body_slot: 1,
            observer_slot: 2,
            retained_stimuli: vec![StimulusType::EventSeesBody],
            body_detectables_after_tick: vec![],
            inform_flag_after_tick: false,
        },
        "an earlier body must broadcast before the later observer detects and consume it"
    );
    assert_eq!(
        observe(false),
        Observation {
            body_before_observer: false,
            body_slot: 2,
            observer_slot: 1,
            retained_stimuli: vec![],
            body_detectables_after_tick: vec![2],
            inform_flag_after_tick: false,
        },
        "a later body may queue next-frame work but must not retroactively rescan an earlier observer"
    );
}

#[test]
fn inline_npc_recovery_precedes_simultaneous_body_inform_and_view() {
    use crate::element::{Camp, Detectable, DetectableType, Entity, EyeStatus};

    let mut engine = EngineInner::new();
    let recovering_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Soldier(recovering) = engine.get_entity_mut(recovering_id).unwrap() else {
        panic!("recovering NPC changed kind")
    };
    recovering.element.active = true;
    recovering.npc.eye_status = EyeStatus::Closed;
    recovering.npc.inform_my_friends = true;
    let ai = recovering.npc.ai_brain.base_mut().unwrap();
    ai.outbox.recovery.inform_resurrection = true;
    ai.outbox.recovery.set_eye_status = Some(EyeStatus::LookForward);

    let Entity::Soldier(observer) = engine.get_entity_mut(observer_id).unwrap() else {
        panic!("observer changed kind")
    };
    observer.element.active = true;
    observer.npc.eye_status = EyeStatus::Closed;
    observer.npc.detectable_lists[DetectableType::Body as usize] = vec![Detectable {
        element: Some(recovering_id),
        detectable_type: DetectableType::Body,
        ..Detectable::default()
    }];

    engine.tick_ai_pending_resurrection_and_eyes_for_npc(recovering_id);

    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for (id, entity) in engine.world.entities.occupied() {
        positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
    }
    crate::sim_rng::with_seed(0x0A01_35A6, |sim| {
        engine.tick_enemy_ai_with_creation_ordered_prelude(sim, &assets, &positions)
    });

    let recovering = engine
        .get_entity(recovering_id)
        .and_then(Entity::npc_data)
        .unwrap();
    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .unwrap();
    assert_eq!(recovering.eye_status, EyeStatus::LookForward);
    assert!(!recovering.inform_my_friends);
    assert!(
        !recovering
            .ai_brain
            .base()
            .unwrap()
            .outbox
            .recovery
            .inform_resurrection
    );
    assert_eq!(
        observer.detectable_lists[DetectableType::Body as usize]
            .iter()
            .map(|detectable| detectable.element)
            .collect::<Vec<_>>(),
        vec![Some(recovering_id)],
        "recovery must delete the stale body first, then the simultaneous inform flag must re-add it"
    );
}

#[test]
#[should_panic(
    expected = "NPC 0 is missing its required AI controller while applying recovery state"
)]
fn npc_recovery_requires_an_ai_controller() {
    let mut engine = EngineInner::new();
    let npc_id = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    engine.tick_ai_pending_resurrection_and_eyes_for_npc(npc_id);
}

#[test]
fn synchronous_look_there_refreshes_only_at_the_receivers_creation_slot() {
    use crate::ai::{
        AiState, CrossNpcAction, Hint, Position, StimulusInfo, StimulusType, Substate,
    };
    use crate::element::{Camp, Entity, EyeStatus};

    fn observe(receiver_before_source: bool) -> (f32, f32, EyeStatus) {
        let mut engine = EngineInner::new();
        let source = make_test_ai_soldier(Camp::Lacklandists);
        let receiver = make_test_ai_soldier(Camp::Lacklandists);
        let (source_id, receiver_id) = if receiver_before_source {
            let receiver_id = engine.add_entity(receiver);
            let source_id = engine.add_entity(source);
            (source_id, receiver_id)
        } else {
            let source_id = engine.add_entity(source);
            let receiver_id = engine.add_entity(receiver);
            (source_id, receiver_id)
        };
        for id in [source_id, receiver_id] {
            let Entity::Soldier(soldier) = engine.get_entity_mut(id).unwrap() else {
                panic!("LOOKTHERE test NPC changed kind")
            };
            soldier.element.active = true;
            soldier.npc.life_points = 100;
            soldier.element.set_direction_instantly(4);
            soldier.npc.direction_old = 4;
        }
        {
            let receiver = engine
                .get_entity_mut(receiver_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap();
            // Recipient selection happened while this soldier was eligible,
            // but an earlier synchronous callback then changed its state.
            // CALL_LOOKTHERE itself is unconditional in the Original.
            receiver.current_state = AiState::Seeking;
            receiver.current_substate = Substate::SeekingGroupCalledByOfficer;
        }
        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);

        let hint = Hint {
            seek_point: Position {
                x: 0.0,
                y: 100.0,
                sector: None,
                level: 0,
            },
            seek_flags: 0,
            who_tells_me: crate::ai::AiEntityHandle::new(source_id.index()),
        };
        engine
            .get_entity_mut(source_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap()
            .outbox
            .reentrant
            .cross_npc_actions
            .push(CrossNpcAction::SendStimulus {
                target: receiver_id.index(),
                stimulus_type: StimulusType::CallLookThere,
                info: StimulusInfo::Hint(hint),
                fallback_to_sender: None,
                to_whole_patrol: false,
            });

        let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        for (id, entity) in engine.world.entities.occupied() {
            positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
        }
        crate::sim_rng::with_seed(0xA013_1007, |sim| {
            if receiver_before_source {
                engine.refresh_npc_view_for_npc(receiver_id, &positions);
                engine.process_synchronous_reentrant_actions_for(sim, source_id, &assets);
            } else {
                engine.process_synchronous_reentrant_actions_for(sim, source_id, &assets);
                engine.refresh_npc_view_for_npc(receiver_id, &positions);
            }
        });

        let receiver = engine
            .get_entity(receiver_id)
            .and_then(Entity::npc_data)
            .unwrap();
        let receiver_ai = receiver.ai_brain.base().unwrap();
        assert_eq!(receiver_ai.current_state, AiState::Wondering);
        assert_eq!(receiver_ai.current_substate, Substate::WonderingWatching);
        (
            receiver.view_angle,
            receiver.view_angle_step,
            receiver.eye_status,
        )
    }

    let earlier = observe(true);
    let later = observe(false);
    assert_eq!(earlier.2, EyeStatus::Stare);
    assert_eq!(later.2, EyeStatus::Stare);
    assert!(
        earlier.0.abs() < f32::EPSILON,
        "an earlier receiver already spent its one RefreshView before LOOKTHERE"
    );
    assert!(
        (later.0 - later.1).abs() < f32::EPSILON,
        "a later receiver must advance its stateful stare exactly once at its own slot"
    );
}

#[test]
fn arrow_reaction_with_null_interesting_object_clears_stale_look_there_focus() {
    use crate::ai::{AiState, CrossNpcAction, Position, StimulusInfo, StimulusType, Substate};
    use crate::element::{Camp, Entity, EyeStatus};

    let mut engine = EngineInner::new();
    let source_id = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let receiver_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    for id in [source_id, receiver_id] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).unwrap() else {
            panic!("arrow-focus test NPC changed kind")
        };
        soldier.element.active = true;
        soldier.npc.life_points = 100;
    }
    {
        let receiver = engine.get_entity_mut(receiver_id).unwrap();
        receiver.npc_data_mut().unwrap().eye_status = EyeStatus::Stare;
        assert_eq!(receiver.enemy_ai().unwrap().base.interesting_object, None);
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .get_entity_mut(source_id)
        .and_then(Entity::ai_controller_mut)
        .unwrap()
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::SendStimulus {
            target: receiver_id.index(),
            stimulus_type: StimulusType::EventGetArrow,
            info: StimulusInfo::Position(Position::default()),
            fallback_to_sender: None,
            to_whole_patrol: false,
        });

    crate::sim_rng::with_seed(0xA013_1091, |sim| {
        engine.process_synchronous_reentrant_actions_for(sim, source_id, &assets);
    });

    let receiver_ai = engine.get_entity(receiver_id).unwrap().enemy_ai().unwrap();
    assert_eq!(receiver_ai.base.current_state, AiState::Seeking);
    assert_eq!(
        receiver_ai.base.current_substate,
        Substate::SeekingArrowReactiontime
    );
    assert_eq!(
        engine
            .get_entity(receiver_id)
            .and_then(Entity::npc_data)
            .unwrap()
            .eye_status,
        EyeStatus::LookForward,
        "Focus(NULL) must unfocus the stale CALL_LOOKTHERE point stare"
    );
}

#[test]
fn look_there_broadcast_skips_attacking_chief_and_reacts_on_eligible_member() {
    use crate::ai::{AiState, CrossNpcAction, LookThereContinuation, Position, Substate};
    use crate::element::{Camp, Entity};

    let mut engine = EngineInner::new();
    let source_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let chief_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let member_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    for id in [source_id, chief_id, member_id] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).unwrap() else {
            panic!("LOOKTHERE broadcast test NPC changed kind")
        };
        soldier.element.active = true;
        soldier.npc.life_points = 100;
        soldier.npc.view_radius = 400;
    }
    {
        let chief = engine
            .get_entity_mut(chief_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap();
        chief.current_state = AiState::Attacking;
        chief.current_substate = Substate::AttackingReactiontime;
    }
    engine
        .get_entity_mut(member_id)
        .and_then(Entity::ai_controller_mut)
        .unwrap()
        .patrol_chief = Some(chief_id);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .get_entity_mut(source_id)
        .and_then(Entity::ai_controller_mut)
        .unwrap()
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::BroadcastLookThere {
            caller: source_id.index(),
            position: Position::default(),
            radius: 100,
            continuation: LookThereContinuation::SeekingArrowReactiontime,
        });

    crate::sim_rng::with_seed(0xA013_1090, |sim| {
        engine.process_synchronous_reentrant_actions_for(sim, source_id, &assets);
    });

    let chief = engine
        .get_entity(chief_id)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert_eq!(chief.current_state, AiState::Attacking);
    assert_eq!(chief.current_substate, Substate::AttackingReactiontime);
    let member = engine
        .get_entity(member_id)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert_eq!(member.current_state, AiState::Wondering);
    assert_eq!(member.current_substate, Substate::WonderingWatching);
}

#[test]
fn subordinate_handles_shadow_locally_when_detected_chief_has_empty_patrol() {
    use crate::ai::{AiState, CrossNpcAction, Position, StimulusInfo, StimulusType, Substate};
    use crate::element::{Camp, Entity};

    let mut engine = EngineInner::new();
    let source_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let chief_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let subordinate_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    for (id, x) in [(source_id, -20.0), (chief_id, 10.0), (subordinate_id, 0.0)] {
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
        assert!(chief.patrol.is_empty());
    }
    {
        let subordinate = engine
            .get_entity_mut(subordinate_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap();
        subordinate.current_state = AiState::Default;
        subordinate.current_substate = Substate::DefaultPatrolEnrouteWaiting;
        subordinate.patrol_chief = Some(chief_id);
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .get_entity_mut(source_id)
        .and_then(Entity::ai_controller_mut)
        .unwrap()
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::SendStimulus {
            target: subordinate_id.index(),
            stimulus_type: StimulusType::EventSeesShadow,
            info: StimulusInfo::Position(Position {
                x: 100.0,
                y: 0.0,
                ..Position::default()
            }),
            fallback_to_sender: None,
            to_whole_patrol: false,
        });

    crate::sim_rng::with_seed(0xA013_2600, |sim| {
        engine.process_synchronous_reentrant_actions_for(sim, source_id, &assets);
    });

    let chief = engine.get_entity(chief_id).unwrap().enemy_ai().unwrap();
    assert_eq!(chief.base.current_state, AiState::Default);
    assert_eq!(chief.base.current_substate, Substate::DefaultOnPost);
    assert_eq!(
        chief
            .last_stimulus_dispatched_to_patrol
            .as_ref()
            .map(|stimulus| stimulus.stimulus_type),
        Some(StimulusType::EventSeesShadow),
        "the empty chief still records the delegated stimulus before returning false"
    );
    let subordinate = engine
        .get_entity(subordinate_id)
        .unwrap()
        .enemy_ai()
        .unwrap();
    assert_eq!(subordinate.base.current_state, AiState::Default);
    assert_eq!(
        subordinate.base.current_substate,
        Substate::DefaultLookingShadow,
        "the subordinate must resume its local handler after the chief returns false"
    );
}

#[test]
fn successful_patrol_dispatch_closes_chief_actor_boundary_before_returning() {
    use crate::ai::{AiState, CrossNpcAction, Position, StimulusInfo, StimulusType, Substate};
    use crate::element::{Camp, Entity};

    let mut engine = EngineInner::new();
    let chief_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let subordinate_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
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
fn enemy_tick_data_populates_live_patrol_chief_without_a_primary_target() {
    use crate::ai::AiState;
    use crate::coordinates::MapPoint;
    use crate::element::Camp;
    use crate::position_interface::SectorHandle;

    let mut engine = EngineInner::new();
    let chief_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let minion_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    {
        let chief = engine.get_entity_mut(chief_id).unwrap();
        chief
            .element_data_mut()
            .set_position_map(MapPoint::new(1042.0, 1783.0));
        chief.element_data_mut().set_layer(2);
        chief.element_data_mut().set_sector(SectorHandle::new(61));
        chief.ai_controller_mut().unwrap().current_state = AiState::Wondering;
    }
    {
        let minion = engine.get_entity_mut(minion_id).unwrap();
        let ai = minion.ai_controller_mut().unwrap();
        ai.patrol_chief = Some(chief_id);
        ai.primary_target = None;
    }
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    crate::sim_rng::with_seed(0xA013_0469, |sim| {
        let scratch = engine.build_sim_scratch(sim, &assets);
        let tick = engine.build_npc_tick_data(sim, minion_id, &scratch, &assets);
        assert_eq!(tick.patrol_chief_position.x, 1042.0);
        assert_eq!(tick.patrol_chief_position.y, 1783.0);
        assert_eq!(tick.patrol_chief_position.level, 2);
        assert_eq!(tick.patrol_chief_position.sector, SectorHandle::new(61));
        assert_eq!(tick.patrol_chief_state, AiState::Wondering);
    });
}

#[test]
fn enemy_tick_data_uses_patrol_chiefs_committed_pass_door_side() {
    use crate::ai::AiState;
    use crate::coordinates::MapPoint;
    use crate::element::{Camp, Command};
    use crate::gate::{Door, DoorIndex, DoorType};
    use crate::sector::SectorNumber;

    let mut engine = EngineInner::new();
    let chief_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let minion_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    {
        let chief = engine.get_entity_mut(chief_id).unwrap();
        chief
            .element_data_mut()
            .set_position_map(MapPoint::new(814.0, 1110.2));
        chief.ai_controller_mut().unwrap().current_state = AiState::Default;
    }
    engine
        .get_entity_mut(minion_id)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .patrol_chief = Some(chief_id);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.script_domains.interactables.doors = vec![Door {
        door_type: DoorType::LiftLow,
        sector_out: SectorNumber::new(89),
        sector_in: SectorNumber::new(96),
        sector_out_index: crate::fast_find_grid::SectorIndex::new(89),
        sector_in_index: crate::fast_find_grid::SectorIndex::new(96),
        point_out: MapPoint::new(821.0, 1124.0),
        point_in: MapPoint::new(811.0, 1103.0),
        layer_out: 2,
        layer_in: 3,
        ..Door::default()
    }];
    let mut pass = crate::sequence::SequenceElement::new_movement(
        1,
        Command::PassDoor,
        Some(chief_id),
        crate::order::OrderType::WalkingStairs,
    );
    if let crate::sequence::SequenceElementData::Movement {
        gate_id, direction, ..
    } = &mut pass.data
    {
        *gate_id = Some(DoorIndex::new(0).expect("valid door index"));
        *direction = 0;
    } else {
        unreachable!("PassDoor fixture must be a movement element")
    }
    crate::sim_rng::with_seed(0xA013_0518, |sim| {
        // This minimal fixture has no installed mission, so entity-view
        // construction intentionally has no canonical door table. Build its
        // otherwise-unrelated scratch snapshot before selecting PassDoor;
        // build_npc_tick_data below must resolve the chief from live state.
        let scratch = engine.build_sim_scratch(sim, &assets);
        let pass_sequence = engine.orders.sequence_manager.launch_element(pass);
        engine
            .orders
            .sequence_manager
            .element_in_progress(pass_sequence, 0);
        let tick = engine.build_npc_tick_data(sim, minion_id, &scratch, &assets);
        assert_eq!(tick.patrol_chief_position.x, 821.0);
        assert_eq!(tick.patrol_chief_position.y, 1124.0);
        assert_eq!(tick.patrol_chief_position.level, 2);
        assert_eq!(
            tick.patrol_chief_position.sector,
            crate::position_interface::SectorHandle::new(89).map(|handle| {
                handle.with_arena_index(crate::fast_find_grid::SectorIndex::new(89).unwrap())
            })
        );
    });
}

#[test]
fn sequence_completion_money_victim_scan_uses_live_off_detection_ko_registry() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::coordinates::MapPoint;
    use crate::element::{Camp, Entity, Posture};
    use crate::sim_rng::{RngSite, with_draw_trace};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let victim_far = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let inactive = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let victim_near = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let dead = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let conscious = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let wrong_camp = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let victim_middle = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let ordinary_ko = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let stale_soldier_slot = engine.add_entity(make_test_civilian(Posture::Upright));

    let fixtures = [
        (owner_id, MapPoint::new(0.0, 0.0), true, 100, false, false),
        (victim_far, MapPoint::new(300.0, 0.0), true, 100, true, true),
        (inactive, MapPoint::new(5.0, 0.0), false, 100, true, true),
        (
            victim_near,
            MapPoint::new(100.0, 0.0),
            true,
            100,
            true,
            true,
        ),
        (dead, MapPoint::new(6.0, 0.0), true, 0, true, true),
        (conscious, MapPoint::new(7.0, 0.0), true, 100, false, true),
        (wrong_camp, MapPoint::new(8.0, 0.0), true, 100, true, true),
        (
            victim_middle,
            MapPoint::new(200.0, 0.0),
            true,
            100,
            true,
            true,
        ),
        (ordinary_ko, MapPoint::new(4.0, 0.0), true, 100, true, false),
    ];
    for (id, position, active, life_points, unconscious, money_fight_ko) in fixtures {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).expect("fixture soldier exists")
        else {
            panic!("fixture changed entity kind")
        };
        soldier.element.active = active;
        soldier.element.posture = if unconscious {
            Posture::Lying
        } else {
            Posture::Upright
        };
        soldier.element.set_position_map(position);
        soldier.npc.life_points = life_points;
        soldier.human.unconscious = unconscious;
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("fixture soldier has Enemy AI");
        ai.base.me = id.index();
        ai.base.knocked_out_in_money_fight = money_fight_ko;
    }
    let owner = engine
        .get_entity_mut(owner_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("owner has Enemy AI");
    owner.base.current_state = AiState::Wondering;
    owner.base.current_substate = Substate::WonderingWatchingForMoreMoney;
    // A sleeping AI substate is not itself unconscious: Original reads the
    // raw `mbUnconscious` flag. Keep this raw-false control out of the list.
    engine
        .get_entity_mut(conscious)
        .and_then(Entity::enemy_ai_mut)
        .expect("sleeping-state control has Enemy AI")
        .base
        .current_substate = Substate::SleepingUnconscious;

    // Deliberately differ from entity-slot order. This is the authored
    // GetSoldier(camp, index) order, including one stale handle whose slot is
    // now occupied by a civilian and must fail current typed validation.
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.ai.global.all_soldier_handles = std::sync::Arc::new(vec![
        victim_middle.index(),
        stale_soldier_slot.index(),
        owner_id.index(),
        inactive.index(),
        victim_far.index(),
        dead.index(),
        wrong_camp.index(),
        victim_near.index(),
        conscious.index(),
        ordinary_ko.index(),
    ]);
    engine.ai.standard_view_polygon_radius = 400;
    let scratch = engine.build_sim_scratch(&sim, &assets);
    let ctx = crate::engine::ai::build_ai_context_from_entity(
        engine.get_entity(owner_id).expect("owner exists"),
        engine.control.frame_counter,
        None,
        engine.world.weather.is_forest_level,
        engine.world.weather.ambiance,
        engine.ai.standard_view_polygon_radius,
        &scratch.ai_entity_views,
        &scratch.ai_sight_obstacles,
        &engine.world.fast_grid,
        &assets.hiking_paths,
        &assets.hiking_waypoint_sectors,
        &engine.ai.global.all_soldier_handles,
        engine.control.sim_config.difficulty,
    );
    let tick = engine.build_npc_tick_data(&sim, owner_id, &scratch, &assets);

    assert_eq!(
        tick.camp_unconscious_soldiers
            .iter()
            .map(|candidate| (candidate.handle, candidate.knocked_out_in_money_fight))
            .collect::<Vec<_>>(),
        vec![
            (victim_middle.index(), true),
            (victim_far.index(), true),
            (victim_near.index(), true),
            (ordinary_ko.index(), false),
        ],
        "off-detection data keeps authored registry order and excludes stale-slot, inactive, dead, raw-conscious, and wrong-camp entries"
    );

    crate::sight_obstacle::begin_parity_visibility_capture();
    let (_, draws) = with_draw_trace(|| {
        engine.dispatch_think_with_drain(
            &sim,
            owner_id,
            &Stimulus::new(StimulusType::EventDone),
            &ctx,
            &tick,
            &assets,
        );
    });
    let queries = crate::sight_obstacle::take_parity_visibility_capture();

    assert_eq!(queries.len(), 3);
    assert_eq!(
        queries
            .iter()
            .map(|query| query.destination[0])
            .collect::<Vec<_>>(),
        vec![200.0, 300.0, 100.0],
        "detection queries retain camp-registry order before distance sorting"
    );
    assert!(
        !draws.contains(&RngSite::MacroRand),
        "a live victim keeps sequence-completion EVENT_DONE out of ReturnToDuty: {draws:?}"
    );
    let owner = engine
        .get_entity(owner_id)
        .and_then(Entity::enemy_ai)
        .expect("owner retains Enemy AI");
    assert_eq!(owner.base.current_state, AiState::Wondering);
    assert_eq!(
        owner.base.current_substate,
        Substate::WonderingApproachingToLoot
    );
    assert_eq!(
        owner.base.detected_body,
        Some(crate::ai::AiEntityHandle::new(victim_near.index()))
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
        let npc_id = engine.add_entity(entity);
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
        entity.element_data_mut().posture = Posture::Lying;
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
        assert_eq!(entity.element_data().posture, Posture::Lying);
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

        let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        for (id, entity) in engine.world.entities.occupied() {
            positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
        }
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
fn frozen_all_does_not_defer_fit_again_recovery_effects() {
    use crate::ai::{AiState, Substate};
    use crate::element::{Camp, Entity, EyeStatus, Posture};

    let mut engine = EngineInner::new();
    let npc_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    engine
        .get_entity_mut(npc_id)
        .unwrap()
        .element_data_mut()
        .active = true;
    complete_test_runtime_fixture(&mut engine, &mut assets);
    std::sync::Arc::make_mut(&mut assets.profile_manager).soldiers[0].wake_up = 1;

    let Entity::Soldier(npc) = engine.get_entity_mut(npc_id).unwrap() else {
        unreachable!()
    };
    npc.element.posture = Posture::Lying;
    npc.human.unconscious = true;
    npc.human.concussion_of_the_brain = crate::combat::CONCUSSION_WAKEUP_THRESHOLD;
    npc.human.concussion_healing_timeout = 0;
    npc.npc.life_points = 100;
    npc.npc.eye_status = EyeStatus::Closed;
    npc.npc.view_radius = 0;
    npc.npc.view_radius_base = 173;
    npc.npc.view_radius_goal = 173;
    npc.npc.view_longrange_radius_factor = 1.0;
    let ai = npc.npc.ai_brain.base_mut().unwrap();
    ai.me = npc_id.index();
    ai.current_state = AiState::Sleeping;
    ai.current_substate = Substate::SleepingUnconscious;

    let observer = engine
        .get_entity_mut(observer_id)
        .unwrap()
        .npc_data_mut()
        .unwrap();
    observer.detectable_lists[crate::element::DetectableType::Body as usize].push(
        crate::element::Detectable {
            element: Some(npc_id),
            detectable_type: crate::element::DetectableType::Body,
            ..Default::default()
        },
    );

    engine.set_actors_frozen(true);
    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for (id, entity) in engine.world.entities.occupied() {
        positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
    }
    crate::sim_rng::with_seed(0x0A01_3F20, |sim| {
        engine.tick_actor_owner_envelopes(sim, &assets, &positions)
    });

    let npc = engine.get_entity(npc_id).unwrap();
    assert_eq!(npc.npc_data().unwrap().eye_status, EyeStatus::LookForward);
    assert!(!npc.human_data().unwrap().unconscious);
    let ai = npc.ai_controller().unwrap();
    assert!(!ai.outbox.recovery.inform_resurrection);
    assert_eq!(ai.outbox.recovery.set_eye_status, None);
    assert!(
        engine
            .get_entity(observer_id)
            .unwrap()
            .npc_data()
            .unwrap()
            .detectable_lists[crate::element::DetectableType::Body as usize]
            .is_empty(),
        "FIT_AGAIN's resurrection fan-out is inline even while FrozenAll skips the NPC tail"
    );
}

#[test]
fn optical_detection_uses_owner_relative_positions_and_spawned_current_fallback() {
    use crate::ai::AiLockFlags;
    use crate::element::{Camp, Detectable, DetectableType, Entity, EyeStatus, Posture};

    fn observed(observer_before_target: bool, spawn_after_snapshot: bool) -> bool {
        let mut engine = EngineInner::new();
        engine.add_entity(Entity::Target(crate::element::ElementTarget {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::Target,
                ..Default::default()
            },
            fx: Default::default(),
            target: Default::default(),
        }));
        let (observer_id, initial_target) = if observer_before_target {
            let observer = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
            let target =
                (!spawn_after_snapshot).then(|| engine.add_entity(make_test_pc(Posture::Upright)));
            (observer, target)
        } else {
            let target = engine.add_entity(make_test_pc(Posture::Upright));
            let observer = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
            (observer, Some(target))
        };
        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        for (id, entity) in engine.world.entities.occupied() {
            positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
        }
        let target_id =
            initial_target.unwrap_or_else(|| engine.add_entity(make_test_pc(Posture::Upright)));
        if spawn_after_snapshot {
            complete_test_runtime_fixture(&mut engine, &mut assets);
        }
        let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .get_mut(0)
            .expect("fixture installs the target character profile");
        profile.detection_speed_in_city = 100;
        profile.detection_speed_in_forest = 100;

        let Entity::Soldier(observer) = engine.get_entity_mut(observer_id).unwrap() else {
            unreachable!()
        };
        observer.element.active = true;
        observer.element.set_position_map(MapPoint::new(0.0, 0.0));
        observer.element.set_direction_instantly(4);
        observer.npc.life_points = 100;
        observer.npc.view_direction = [1.0, 0.0];
        observer.npc.view_radius = 200;
        observer.npc.view_radius_base = 200;
        observer.npc.view_radius_goal = 200;
        observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
        observer.npc.eye_status = EyeStatus::Stare;
        observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
        observer.npc.detectable_lists[DetectableType::Enemy as usize] = vec![Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            shadow_seen_last_frame: true,
            ..Default::default()
        }];
        let ai = observer.npc.ai_brain.base_mut().unwrap();
        ai.me = observer_id.index();
        ai.locks_flag_field = AiLockFlags::FREEZE;

        let before = MapPoint::new(50.0, 0.0);
        let after = if spawn_after_snapshot {
            MapPoint::new(50.0, 0.0)
        } else {
            MapPoint::new(5_000.0, 0.0)
        };
        if positions.get(target_id).is_some() {
            positions[target_id] = Some(crate::entities::BoundaryPosition {
                map: before,
                world: crate::coordinates::WorldPoint3D::new(before.x, before.y, 0.0),
            });
        }
        let Entity::Pc(target) = engine.get_entity_mut(target_id).unwrap() else {
            unreachable!()
        };
        target.element.active = true;
        target.element.set_position_map(after);
        target.pc.life_points = 100;

        let sim = crate::sim_rng::test_context();
        let mut prepared = engine.prepare_npc_owner_pass(&sim, &assets);
        engine.tick_npc_owner_pass(&sim, &assets, &positions, &mut prepared, observer_id);

        engine
            .get_entity(observer_id)
            .unwrap()
            .npc_data()
            .unwrap()
            .detectable_lists[DetectableType::Enemy as usize][0]
            .seen_last_frame
    }

    assert!(
        observed(true, false),
        "an earlier observer must see a later moving target at its pre-movement position"
    );
    assert!(
        !observed(false, false),
        "a later observer must see an earlier moving target at its post-movement position"
    );
    assert!(
        observed(true, true),
        "a callback-spawned later target absent from the oracle must use its live current position"
    );
}

#[test]
fn dispatch_ai_stimulus_intentionally_ignores_pcs() {
    use crate::ai::{Stimulus, StimulusType};
    use crate::element::{Entity, Posture};

    let mut engine = EngineInner::new();
    let pc_id = engine.add_entity(make_test_pc(Posture::Upright));

    engine.dispatch_ai_stimulus(pc_id, Stimulus::new(StimulusType::EventFitAgain));

    assert!(matches!(engine.get_entity(pc_id), Some(Entity::Pc(_))));
}

#[test]
fn wake_prefix_preserves_existing_stimulus_fifo() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::element::{Camp, Entity, EyeStatus};

    let mut engine = EngineInner::new();
    let npc_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let ai = engine
        .get_entity_mut(npc_id)
        .and_then(Entity::ai_controller_mut)
        .unwrap();
    ai.current_state = AiState::Default;
    ai.outbox.detection.stimuli = vec![
        Stimulus::new(StimulusType::EventLoseConsciousness),
        Stimulus::new(StimulusType::EventFitAgain),
        Stimulus::new(StimulusType::EventImpossible),
    ];

    let woke = crate::sim_rng::with_seed(0xA013_F1F0, |sim| {
        engine.dispatch_pending_fit_again_for_npc(sim, npc_id, &assets)
    });
    assert!(woke);
    let ai = engine
        .get_entity(npc_id)
        .and_then(Entity::ai_controller)
        .unwrap();
    assert_eq!(
        ai.outbox
            .detection
            .stimuli
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![StimulusType::EventImpossible],
        "the older prefix through FITAGAIN must dispatch in FIFO order while only the suffix remains"
    );
    assert_eq!(ai.current_state, AiState::Sleeping);
    assert_eq!(
        ai.current_substate,
        Substate::SleepingAwakening,
        "LOSE_CONSCIOUSNESS must run before FITAGAIN; plucking FITAGAIN first would leave the NPC unconscious"
    );
    assert_eq!(
        ai.outbox.recovery.set_eye_status, None,
        "each synchronous Think in the restored FIFO prefix must commit its eye write inline"
    );
    assert_eq!(
        engine
            .get_entity(npc_id)
            .and_then(Entity::npc_data)
            .unwrap()
            .eye_status,
        EyeStatus::LookForward,
        "LOSE_CONSCIOUSNESS and the following FITAGAIN must publish their SetViewStatus writes in FIFO order"
    );
}

#[test]
fn restored_quit_lose_quit_fifo_commits_unconscious_eyes_inline() {
    use crate::ai::{Stimulus, StimulusType};
    use crate::element::{Camp, Entity, EyeStatus};

    let mut engine = EngineInner::new();
    let npc_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let ai = engine
        .get_entity_mut(npc_id)
        .and_then(Entity::ai_controller_mut)
        .unwrap();
    ai.outbox.detection.stimuli = vec![
        Stimulus::new(StimulusType::EventQuitSwordfight),
        Stimulus::new(StimulusType::EventLoseConsciousness),
        Stimulus::new(StimulusType::EventQuitSwordfight),
    ];

    crate::sim_rng::with_seed(0xA013_105E, |sim| {
        engine.tick_enemy_ai_drain_pending_stimuli_for_npc(sim, npc_id, &assets, None, None)
    });

    let entity = engine.get_entity(npc_id).unwrap();
    assert_eq!(
        entity.npc_data().unwrap().eye_status,
        EyeStatus::DieOrGetUnconscious,
        "the middle LOSE_CONSCIOUSNESS Think must publish its eye write despite the surrounding restored FIFO prefix/suffix"
    );
    assert_eq!(
        entity
            .ai_controller()
            .unwrap()
            .outbox
            .recovery
            .set_eye_status,
        None,
        "the restored FIFO must not strand its synchronous SetViewStatus write"
    );
}

#[test]
fn wake_blinks_apply_inline_at_the_waker_slot_for_both_producers() {
    use crate::ai::{AiState, StimulusType, Substate};
    use crate::combat::ConcussionOutcome;
    use crate::element::{Camp, Detectable, DetectableType, Entity, EyeStatus, Posture};

    type BlinkState = (bool, bool);

    fn observe(waker_before_observer: bool, natural: bool) -> (BlinkState, BlinkState) {
        let mut engine = EngineInner::new();
        engine.ai.global.there_are_royalist_soldiers = true;
        engine.ai.global.there_are_lacklandist_soldiers = true;
        let waker = make_test_ai_soldier(Camp::Royalists);
        let observer = make_test_ai_soldier(Camp::Lacklandists);
        let (waker_id, observer_id) = if waker_before_observer {
            (engine.add_entity(waker), engine.add_entity(observer))
        } else {
            let observer_id = engine.add_entity(observer);
            let waker_id = engine.add_entity(waker);
            (waker_id, observer_id)
        };
        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles
            .soldiers
            .resize_with(1, crate::profiles::SoldierProfile::default);
        profiles.soldiers[0].wake_up = 1;

        let Entity::Soldier(waker) = engine.get_entity_mut(waker_id).unwrap() else {
            unreachable!()
        };
        waker.element.active = true;
        waker.element.posture = Posture::Lying;
        waker.npc.life_points = 100;
        waker.npc.eye_status = EyeStatus::Closed;
        let ai = waker.npc.ai_brain.base_mut().unwrap();
        ai.script_locked = false;
        ai.current_state = AiState::Sleeping;
        ai.current_substate = Substate::SleepingUnconscious;
        if natural {
            waker.human.unconscious = true;
            waker.human.concussion_of_the_brain = crate::combat::CONCUSSION_WAKEUP_THRESHOLD;
            waker.human.concussion_healing_timeout = 0;
        }
        let Entity::Soldier(observer) = engine.get_entity_mut(observer_id).unwrap() else {
            unreachable!()
        };
        observer.element.active = false;
        observer.npc.detectable_lists[DetectableType::Enemy as usize] = vec![Detectable {
            element: Some(waker_id),
            detectable_type: DetectableType::Enemy,
            seen_now: true,
            seen_last_frame: true,
            ..Detectable::default()
        }];

        if natural {
            engine.tick_concussion_healing(&assets);
        } else {
            engine
                .orders
                .pending_concussion_side_effects
                .push((waker_id, ConcussionOutcome::WokeUp));
            crate::sim_rng::with_seed(0x0A01_3B11, |sim| {
                engine.drain_pending_concussion_side_effects(sim, &assets)
            });
        }
        assert!(
            engine
                .get_entity(waker_id)
                .and_then(Entity::ai_controller)
                .unwrap()
                .outbox
                .detection
                .stimuli
                .iter()
                .any(|stimulus| stimulus.stimulus_type == StimulusType::EventFitAgain),
            "producer natural={natural}, waker_before_observer={waker_before_observer}, unconscious={}",
            engine
                .get_entity(waker_id)
                .and_then(Entity::human_data)
                .unwrap()
                .unconscious
        );

        let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        for (id, entity) in engine.world.entities.occupied() {
            positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
        }
        crate::sim_rng::with_seed(0x0A01_3B12, |sim| {
            engine.tick_enemy_ai_with_creation_ordered_prelude(sim, &assets, &positions)
        });

        let snapshot = |engine: &EngineInner| {
            let observer = engine.get_entity(observer_id).unwrap();
            let detectable =
                &observer.npc_data().unwrap().detectable_lists[DetectableType::Enemy as usize][0];
            (detectable.seen_now, detectable.seen_last_frame)
        };
        let first_slot = snapshot(&engine);

        crate::sim_rng::with_seed(0x0A01_3B13, |sim| {
            engine.tick_enemy_ai_with_creation_ordered_prelude(sim, &assets, &positions)
        });
        let next_slot = snapshot(&engine);

        (first_slot, next_slot)
    }

    for natural in [true, false] {
        assert_eq!(observe(true, natural), ((false, false), (false, false)));
        assert_eq!(
            observe(false, natural),
            ((false, false), (false, false)),
            "BlinkEnemy must mutate an already-visited opposing observer inline at the later waker's slot"
        );
    }
}

#[test]
fn npc_detection_observes_friend_state_at_creation_order_boundary() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};
    use crate::profiles::ProfileRank;

    fn observe(attacker_before_officer: bool) -> (AiState, AiState, Substate) {
        let mut engine = EngineInner::new();
        // Keep the relevant NPCs in slots 1/2 in both arrangements so the
        // swapped oracle is not confounded by a slot-zero special case.
        engine.add_entity(Entity::Target(crate::element::ElementTarget {
            element: ElementData {
                kind: ElementKind::Target,
                ..ElementData::default()
            },
            fx: Default::default(),
            target: Default::default(),
        }));

        let attacker = make_test_ai_soldier(Camp::Lacklandists);
        let officer = make_test_ai_soldier(Camp::Lacklandists);
        let (attacker_id, officer_id) = if attacker_before_officer {
            (engine.add_entity(attacker), engine.add_entity(officer))
        } else {
            let officer_id = engine.add_entity(officer);
            let attacker_id = engine.add_entity(attacker);
            (attacker_id, officer_id)
        };
        let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

        for (id, x) in [(officer_id, 0.0), (attacker_id, 120.0)] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("creation-order detection soldier exists")
            else {
                panic!("creation-order detection entity changed kind")
            };
            soldier.element.active = true;
            soldier
                .element
                .set_position(crate::coordinates::WorldPoint3D { x, y: 0.0, z: 0.0 });
            soldier.element.set_position_map(MapPoint::new(x, 0.0));
            soldier.element.set_direction_instantly(4);
            soldier.npc.life_points = 100;
            soldier.npc.view_direction = [1.0, 0.0];
            soldier.npc.view_radius = 135;
            soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
            soldier.npc.eye_status = crate::element::EyeStatus::Stare;
            let ai = soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("creation-order detection soldier has enemy AI");
            ai.base.me = id.index();
            ai.soldier_profile_rank = if id == officer_id {
                ProfileRank::Officer
            } else {
                ProfileRank::Soldier
            };
        }

        let Entity::Pc(pc) = engine
            .get_entity_mut(pc_id)
            .expect("creation-order detection PC exists")
        else {
            panic!("creation-order detection target changed kind")
        };
        pc.element.active = true;
        pc.element.set_position(crate::coordinates::WorldPoint3D {
            x: 175.0,
            y: 0.0,
            z: 0.0,
        });
        pc.element.set_position_map(MapPoint::new(175.0, 0.0));
        pc.pc.life_points = 100;

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .get_mut(0)
            .expect("fixture installs the PC character profile");
        profile.detection_speed_in_city = 100;
        profile.detection_speed_in_forest = 100;

        // Isolate the exact A retained-EVENT_VIEW tail → B-FRIEND edge after
        // fixture initialization has installed profiles and AI defaults.
        let Entity::Soldier(attacker) = engine
            .get_entity_mut(attacker_id)
            .expect("attacker exists before detection")
        else {
            panic!("attacker changed kind")
        };
        attacker.npc.detectable_lists[DetectableType::Enemy as usize].clear();
        attacker.npc.detectable_lists[DetectableType::Friend as usize].clear();
        attacker
            .npc
            .ai_brain
            .base_mut()
            .expect("attacker retains base AI")
            .stimulus_queue
            .push(Stimulus::with_human(StimulusType::EventView, pc_id.index()));

        let Entity::Soldier(officer) = engine
            .get_entity_mut(officer_id)
            .expect("officer exists before detection")
        else {
            panic!("officer changed kind")
        };
        officer.npc.detectable_lists[DetectableType::Friend as usize].clear();
        officer.npc.detectable_lists[DetectableType::Friend as usize].push(Detectable {
            element: Some(attacker_id),
            detectable_type: DetectableType::Friend,
            ..Detectable::default()
        });

        let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        for (id, entity) in engine.world.entities.occupied() {
            positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
        }
        crate::sim_rng::with_seed(0xA013, |sim| {
            engine.tick_enemy_ai_with_creation_ordered_prelude(sim, &assets, &positions)
        });

        let attacker_ai = engine
            .get_entity(attacker_id)
            .and_then(Entity::enemy_ai)
            .expect("attacker remains an enemy AI");
        let officer_ai = engine
            .get_entity(officer_id)
            .and_then(Entity::enemy_ai)
            .expect("officer remains an enemy AI");
        (
            attacker_ai.base.current_state,
            officer_ai.base.current_state,
            officer_ai.base.current_substate,
        )
    }

    let attacker_first = observe(true);
    assert_eq!(attacker_first.0, AiState::Attacking);
    assert_eq!(
        attacker_first.1,
        AiState::Default,
        "later officer must see that the earlier EVENT_VIEW made its friend unable to help"
    );
    assert_ne!(attacker_first.2, Substate::SeekingOfficerCallSoldier);
    // The officer already faces the helpful soldier, so the Face inside the
    // FRIEND sighting is a no-op and the Think tail posts EVENT_DONE
    // synchronously: CallSoldier hails the soldier in the same slot and the
    // officer ends the tick already waiting for him.
    assert_eq!(
        observe(false),
        (
            AiState::Attacking,
            AiState::Seeking,
            Substate::SeekingOfficerWaitForSoldier,
        ),
        "earlier officer must see the still-helpful soldier before that soldier handles EVENT_VIEW"
    );
}

#[test]
fn npc_hearing_thinks_before_same_slot_optical_detection() {
    use crate::ai::AiState;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    // Keep the NPC out of legacy slot zero and choose the frame so its
    // `(frame + creation_order) % DETECTION_FREQUENCY_SOUNDS` gate is open.
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: ElementData {
            kind: ElementKind::Target,
            ..ElementData::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    // Slot 1 has Original creation order 32 after the hidden pre-level
    // prefix, so frame 1 opens its three-frame hearing cadence.
    engine.control.frame_counter = 1;

    let soldier_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let stale_pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Dead));

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("hearing-order soldier exists")
    else {
        panic!("hearing-order entity changed kind")
    };
    soldier.element.active = true;
    soldier
        .element
        .set_position(crate::coordinates::WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
    soldier.element.set_position_map(MapPoint::new(0.0, 0.0));
    soldier.element.set_direction_instantly(4);
    soldier.npc.life_points = 100;
    soldier.npc.view_direction = [1.0, 0.0];
    soldier.npc.view_radius = 135;
    soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    soldier.npc.eye_status = crate::element::EyeStatus::Stare;
    let ai = soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("hearing-order soldier has enemy AI");
    ai.base.me = soldier_id.index();

    let Entity::Pc(pc) = engine
        .get_entity_mut(pc_id)
        .expect("hearing-order PC exists")
    else {
        panic!("hearing-order target changed kind")
    };
    pc.element.active = true;
    pc.element.set_position(crate::coordinates::WorldPoint3D {
        x: 55.0,
        y: 0.0,
        z: 0.0,
    });
    pc.element.set_position_map(MapPoint::new(55.0, 0.0));
    pc.pc.life_points = 100;

    let Entity::Pc(stale_pc) = engine
        .get_entity_mut(stale_pc_id)
        .expect("stale hearing-order PC exists")
    else {
        panic!("stale hearing-order target changed kind")
    };
    stale_pc.element.active = false;
    stale_pc.pc.life_points = 0;

    // RunningUpright on ground produces a 70-volume TAPTAPTAP. At 55 units
    // this becomes the original's 15-volume subjective noise, while keeping
    // EVENT_VIEW outside the unrelated close-combat branch (< 50 units).
    // Install it as the production snapshot builder's current animation
    // instead of injecting a synthetic noise into the detection helper.
    let mut movement = SequenceElement::new_movement(
        1,
        crate::element::Command::Move,
        Some(pc_id),
        OrderType::RunningUpright,
    );
    movement
        .orders
        .push_back(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("soldier exists before hearing-order detection")
    else {
        panic!("hearing-order soldier changed kind")
    };
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    soldier.npc.detection_suspects[DetectableType::Enemy as usize] = 0;
    // Acoustics runs before optical CleanUpDetectables. A just-dead PC can
    // therefore still occupy an earlier enemy-list slot without appearing in
    // the alive-only world snapshot; it must not block the later audible PC.
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(stale_pc_id),
        detectable_type: DetectableType::Enemy,
        ..Detectable::default()
    });
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(pc_id),
        detectable_type: DetectableType::Enemy,
        // Keep the oracle about HEAR → VIEW rather than predetection shadow.
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0xA013_0EAD, |sim| engine.tick_enemy_ai(sim, &assets));

    assert_eq!(
        engine
            .get_entity(pc_id)
            .and_then(Entity::actor_data)
            .expect("hearing-order PC remains an actor")
            .last_noise_volume,
        70,
        "production PC snapshot must derive the expected running noise"
    );
    assert!(
        engine
            .get_entity(soldier_id)
            .and_then(Entity::npc_data)
            .expect("hearing-order soldier remains an NPC")
            .detectable_lists[DetectableType::Enemy as usize][0]
            .heard_last_frame,
        "production acoustic pass must reach UpdateHearing's latch"
    );
    let ai = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("soldier remains an enemy AI");
    assert_eq!(
        ai.base.current_state,
        AiState::Attacking,
        "synchronous EVENT_HEAR must make optical detection instant in the same RefreshDetection call"
    );
}

#[test]
fn detection_tick_preserves_authoritative_enemy_membership() {
    use crate::element::{Camp, DetectableType, Entity};

    let mut engine = EngineInner::new();
    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("membership observer exists")
    else {
        panic!("membership observer changed kind")
    };
    observer.element.active = true;
    observer.npc.life_points = 100;

    let Entity::Pc(pc) = engine.get_entity_mut(pc_id).expect("untracked PC exists") else {
        panic!("untracked target changed kind")
    };
    pc.element.active = true;
    pc.pc.life_points = 100;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("membership observer exists after fixture")
    else {
        panic!("membership observer changed kind after fixture")
    };
    observer.npc.detectable_lists[DetectableType::Enemy as usize].clear();

    // Slot zero has Original creation order 31. Frame two opens the modulo-3
    // acoustic gate as well as running the optical pass, so this one tick
    // exercises both places that formerly reconciled every missing PC.
    engine.control.frame_counter = 2;
    crate::sim_rng::with_seed(0xA013_0EAE, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("membership observer remains an NPC");
    assert!(
        observer.detectable_lists[DetectableType::Enemy as usize].is_empty(),
        "RefreshDetection must only iterate serialized/explicitly-added detectables; it must not synthesize a missing PC"
    );
}

#[test]
fn lackland_detection_scans_and_retains_full_fifo_while_ai_locked() {
    use crate::ai::{AiLockFlags, AiState, StimulusInfo, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{
        Camp, Detectable, DetectableType, ElementBonus, ElementData, ElementKind, Entity,
    };
    use crate::element_kinds::ObjectType;
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: ElementData {
            kind: ElementKind::Target,
            ..ElementData::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    // The observer's modified frame is universal frame + its creation order
    // (31 hidden pre-mission elements, then the Target, so the observer sits
    // at 32). Every strict per-bucket cadence in this oracle — hearing (3),
    // Body (8), Object (4), Enemy-PC (2) — must be open in the same tick,
    // so pick a frame with 16 + 32 = 48 ≡ 0 mod lcm(3, 8, 4, 2) = 24.
    engine.control.frame_counter = 16;

    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let first_visible_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let lost_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let last_visible_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let body_id = engine.add_entity(make_test_pc(crate::element::Posture::Dead));
    let object_id = engine.add_entity(Entity::Bonus(ElementBonus {
        element: ElementData {
            kind: ElementKind::ObjectBonus,
            active: true,
            ..ElementData::default()
        },
        object: crate::element::ObjectData {
            object_type: ObjectType::Coin,
            ..crate::element::ObjectData::default()
        },
    }));
    let friend_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("locked detection observer exists")
    else {
        panic!("locked detection observer changed kind")
    };
    observer.element.active = true;
    observer
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    observer.element.set_position_map(MapPoint::new(0.0, 0.0));
    observer.element.set_direction_instantly(4);
    observer.npc.life_points = 100;
    observer.npc.view_direction = [1.0, 0.0];
    observer.npc.view_radius = 300;
    observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    observer.npc.eye_status = crate::element::EyeStatus::Stare;

    for (id, x, life_points) in [
        (first_visible_id, 55.0, 100),
        (lost_id, -200.0, 100),
        (last_visible_id, 80.0, 100),
        (body_id, 100.0, 0),
    ] {
        let Entity::Pc(pc) = engine
            .get_entity_mut(id)
            .expect("locked detection PC exists")
        else {
            panic!("locked detection PC changed kind")
        };
        pc.element.active = true;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(x, 0.0));
        pc.pc.life_points = life_points;
    }

    let Entity::Bonus(object) = engine
        .get_entity_mut(object_id)
        .expect("locked detection object exists")
    else {
        panic!("locked detection object changed kind")
    };
    object
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(100.0, 0.0, 0.0));
    object.element.set_position_map(MapPoint::new(100.0, 0.0));

    let Entity::Soldier(friend) = engine
        .get_entity_mut(friend_id)
        .expect("locked observer's friend exists")
    else {
        panic!("locked observer's friend changed kind")
    };
    friend.element.active = true;
    friend
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(-20.0, 20.0, 0.0));
    friend.element.set_position_map(MapPoint::new(-20.0, 20.0));
    friend.npc.life_points = 100;
    friend.npc.eye_status = crate::element::EyeStatus::Closed;

    // RunningUpright produces the production 70-volume TAPTAPTAP used by
    // RefreshDetection's acoustic pass; the frame chosen above keeps the
    // observer's three-frame hearing cadence open.
    let mut movement = SequenceElement::new_movement(
        1,
        crate::element::Command::Move,
        Some(first_visible_id),
        OrderType::RunningUpright,
    );
    movement
        .orders
        .push_back(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("locked detection observer exists after fixture")
    else {
        panic!("locked detection observer changed kind after fixture")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("locked detection observer has enemy AI");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::FREEZE;

    observer.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    observer.npc.detectable_lists[DetectableType::Body as usize].clear();
    observer.npc.detectable_lists[DetectableType::Object as usize].clear();
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    observer.npc.detection_suspects[DetectableType::Body as usize] = 999;
    observer.npc.detection_suspects[DetectableType::Object as usize] = 999;
    for (target_id, seen_last_frame) in [
        (first_visible_id, false),
        (lost_id, true),
        (last_visible_id, false),
    ] {
        observer.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            seen_last_frame,
            ..Detectable::default()
        });
    }
    observer.npc.detectable_lists[DetectableType::Body as usize].push(Detectable {
        element: Some(body_id),
        detectable_type: DetectableType::Body,
        // Keep this oracle's shadow prefix confined to the Enemy bucket.
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });
    observer.npc.detectable_lists[DetectableType::Object as usize].push(Detectable {
        element: Some(object_id),
        detectable_type: DetectableType::Object,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0xA013_0B22, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("locked detection observer remains an NPC");
    assert_eq!(
        observer.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .map(|det| (
                det.heard_last_frame,
                det.shadow_seen_last_frame,
                det.seen_last_frame,
            ))
            .collect::<Vec<_>>(),
        vec![
            (true, true, true),
            (false, false, false),
            (false, true, true)
        ],
        "AI lock must not suppress acoustic, predetection, or optical latch updates"
    );
    assert_eq!(
        observer.detection_suspects[DetectableType::Enemy as usize],
        0,
        "locked Enemy detection must still commit and reset suspects"
    );
    assert!(
        observer.detectable_lists[DetectableType::Body as usize].is_empty(),
        "locked non-Enemy buckets must still commit one-shot detectables"
    );
    assert!(
        observer.detectable_lists[DetectableType::Object as usize].is_empty(),
        "locked Object detection must still commit its one-shot detectable"
    );

    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("locked detection observer retains enemy AI");
    assert_eq!(
        (ai.base.current_state, ai.base.current_substate),
        (AiState::Default, Substate::DefaultOnPost)
    );
    assert!(ai.base.outbox.detection.stimuli.is_empty());
    assert_eq!(
        ai.base.last_stimulus_actor,
        Some(crate::ai::AiEntityHandle::new(body_id.index()))
    );
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![
            StimulusType::EventHear,
            StimulusType::EventSeesShadow,
            StimulusType::EventSeesShadow,
            StimulusType::EventView,
            StimulusType::EventOutOfView,
            StimulusType::EventView,
            StimulusType::EventSeesBody,
            StimulusType::EventSeesObject,
        ],
        "StartThink must retain the complete HEAR then optical FIFO under AI lock"
    );
    assert!(matches!(
        ai.base.stimulus_queue[0].info,
        StimulusInfo::Noise(_)
    ));
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .filter_map(|stimulus| match stimulus.info {
                StimulusInfo::Human(target) => Some(target),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            crate::ai::AiEntityHandle::new(first_visible_id.index()),
            crate::ai::AiEntityHandle::new(lost_id.index()),
            crate::ai::AiEntityHandle::new(last_visible_id.index()),
            crate::ai::AiEntityHandle::new(body_id.index()),
        ]
    );
    assert_eq!(
        ai.base
            .stimulus_queue
            .last()
            .expect("Object event closes the retained detection FIFO")
            .info,
        StimulusInfo::Object(crate::ai::AiEntityHandle::new(object_id.index())),
        "EVENT_SEES_OBJECT must retain an object payload, not impersonate a human"
    );
    assert_eq!(
        engine
            .get_entity(friend_id)
            .and_then(Entity::npc_data)
            .expect("locked observer's friend remains an NPC")
            .ai_state(),
        AiState::Default,
        "a retained VIEW must not leak through the later out-of-band ally alert"
    );
    assert!(
        !engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("locked detection observer remains an NPC")
            .alerted,
        "AILOCK_FREEZE must retain VIEW without pre-alerting its observer"
    );

    // Static RHArtificialIntelligence::mbFreeze is a separate mode: the
    // next RefreshDetection still scans and commits its latch, but StartThink
    // discards the resulting VIEW instead of retaining it.
    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("static-freeze detection observer exists")
    else {
        panic!("static-freeze detection observer changed kind")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("static-freeze detection observer retains enemy AI");
    ai.base.locks_flag_field = AiLockFlags::empty();
    ai.base.stimulus_queue.clear();
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    observer.npc.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame = false;
    observer.npc.detectable_lists[DetectableType::Enemy as usize][0].shadow_seen_last_frame = true;

    engine.ai.global.freeze = true;
    crate::sim_rng::with_seed(0xA013_0B24, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("static-freeze detection observer remains an NPC");
    assert!(
        observer.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame,
        "static AI freeze must not suppress RefreshDetection latch commits"
    );
    let ai = observer
        .ai_brain
        .enemy()
        .expect("static-freeze detection observer retains enemy AI");
    assert!(
        ai.base.stimulus_queue.is_empty(),
        "static AI freeze must discard detection stimuli"
    );
    assert_eq!(ai.base.current_state, AiState::Default);
    assert!(!observer.alerted);
}

#[test]
fn inactive_building_viewer_runs_hearing_then_optics_while_outdoor_viewer_is_a_noop() {
    use crate::ai::{AiLockFlags, AiState, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    for observer_inside in [true, false] {
        let mut engine = EngineInner::new();
        // Slot 0 has Original creation order 31; frame 2 opens its
        // three-frame hearing cadence.
        engine.control.frame_counter = 2;
        let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
        let indoor_target_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
        let inactive_outdoor_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
        let runner_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
        let building = crate::position_interface::SectorHandle::new(42).unwrap();
        install_test_building_sector(&mut engine, 42);

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("inactive-viewer observer exists")
        else {
            panic!("inactive-viewer observer changed kind")
        };
        observer.element.active = true;
        observer
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
        observer.element.set_position_map(MapPoint::new(0.0, 0.0));
        observer.element.set_direction_instantly(4);
        observer.npc.life_points = 100;
        observer.npc.view_direction = [1.0, 0.0];
        observer.npc.view_radius = 300;
        observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
        observer.npc.eye_status = crate::element::EyeStatus::Stare;
        if observer_inside {
            observer.element.set_sector(Some(building));
        }

        let Entity::Pc(indoor_target) = engine
            .get_entity_mut(indoor_target_id)
            .expect("inactive same-building target exists")
        else {
            panic!("inactive same-building target changed kind")
        };
        indoor_target.element.active = true;
        indoor_target
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(40.0, 0.0, 0.0));
        indoor_target
            .element
            .set_position_map(MapPoint::new(40.0, 0.0));
        indoor_target.element.set_sector(Some(building));
        indoor_target.pc.life_points = 100;

        let Entity::Pc(inactive_outdoor) = engine
            .get_entity_mut(inactive_outdoor_id)
            .expect("inactive outdoor target exists")
        else {
            panic!("inactive outdoor target changed kind")
        };
        inactive_outdoor.element.active = true;
        inactive_outdoor
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(45.0, 0.0, 0.0));
        inactive_outdoor
            .element
            .set_position_map(MapPoint::new(45.0, 0.0));
        inactive_outdoor.pc.life_points = 100;

        let Entity::Pc(runner) = engine
            .get_entity_mut(runner_id)
            .expect("inactive-viewer runner exists")
        else {
            panic!("inactive-viewer runner changed kind")
        };
        runner.element.active = true;
        runner
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(55.0, 0.0, 0.0));
        runner.element.set_position_map(MapPoint::new(55.0, 0.0));
        runner.pc.life_points = 100;

        let mut movement = SequenceElement::new_movement(
            1,
            crate::element::Command::Move,
            Some(runner_id),
            OrderType::RunningUpright,
        );
        movement
            .orders
            .push_back(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
        let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(movement_sequence, 0);

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .get_mut(0)
            .expect("fixture installs the PC character profile");
        profile.detection_speed_in_city = 100;
        profile.detection_speed_in_forest = 100;

        let Entity::Pc(indoor_target) = engine
            .get_entity_mut(indoor_target_id)
            .expect("same-building target exists after fixture")
        else {
            panic!("same-building target changed kind after fixture")
        };
        indoor_target.element.active = false;
        engine
            .get_entity_mut(inactive_outdoor_id)
            .expect("inactive outdoor target exists after fixture")
            .element_data_mut()
            .active = false;

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("inactive-viewer observer exists after fixture")
        else {
            panic!("inactive-viewer observer changed kind after fixture")
        };
        observer.element.active = false;
        let ai = observer
            .npc
            .ai_brain
            .enemy_mut()
            .expect("inactive-viewer observer has enemy AI");
        ai.base.me = observer_id.index();
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultOnPost;
        ai.current_task_priority = task_priority::NONE;
        ai.base.locks_flag_field = AiLockFlags::BUSY;
        observer.npc.detectable_lists[DetectableType::Enemy as usize] = vec![
            Detectable {
                element: Some(indoor_target_id),
                detectable_type: DetectableType::Enemy,
                shadow_seen_last_frame: true,
                ..Detectable::default()
            },
            Detectable {
                element: Some(inactive_outdoor_id),
                detectable_type: DetectableType::Enemy,
                seen_last_frame: true,
                shadow_seen_last_frame: true,
                ..Detectable::default()
            },
            Detectable {
                element: Some(runner_id),
                detectable_type: DetectableType::Enemy,
                shadow_seen_last_frame: true,
                ..Detectable::default()
            },
        ];
        observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
        observer.npc.maximal_detection_suspect = 777;

        crate::sim_rng::with_seed(0xA013_1A51, |sim| engine.tick_enemy_ai(sim, &assets));

        let observer = engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("inactive-viewer observer remains an NPC");
        let ai = observer
            .ai_brain
            .enemy()
            .expect("inactive-viewer observer retains enemy AI");
        assert_eq!(ai.base.current_state, AiState::Default);

        if observer_inside {
            assert_eq!(
                ai.base
                    .stimulus_queue
                    .iter()
                    .map(|stimulus| stimulus.stimulus_type)
                    .collect::<Vec<_>>(),
                vec![
                    StimulusType::EventHear,
                    StimulusType::EventView,
                    StimulusType::EventOutOfView,
                ],
                "an inactive building viewer must retain acoustic-before-optical FIFO order"
            );
            assert!(observer.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame);
            assert!(
                !observer.detectable_lists[DetectableType::Enemy as usize][1].seen_last_frame,
                "a living inactive outdoor PC must stay in the list and produce a falling edge"
            );
            assert!(observer.detectable_lists[DetectableType::Enemy as usize][2].heard_last_frame);
            assert_eq!(
                observer.detection_suspects[DetectableType::Enemy as usize],
                0
            );
            assert_eq!(observer.maximal_detection_suspect, 0);
            assert_eq!(
                engine
                    .get_entity(indoor_target_id)
                    .and_then(Entity::actor_data)
                    .expect("inactive indoor target remains an actor")
                    .last_noise_volume,
                0,
                "retaining an inactive PC for same-building sight must not make it audible"
            );
        } else {
            assert!(
                ai.base.stimulus_queue.is_empty(),
                "the first RefreshDetection gate must make an inactive outdoor viewer a no-op"
            );
            assert_eq!(
                observer.detectable_lists[DetectableType::Enemy as usize].len(),
                3,
                "living inactive targets remain Enemy detectables until they die"
            );
            assert!(!observer.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame);
            assert!(observer.detectable_lists[DetectableType::Enemy as usize][1].seen_last_frame);
            assert!(!observer.detectable_lists[DetectableType::Enemy as usize][2].heard_last_frame);
            assert_eq!(
                observer.detection_suspects[DetectableType::Enemy as usize],
                999
            );
            assert_eq!(observer.maximal_detection_suspect, 777);
        }
    }
}

#[test]
fn inactive_npc_blip_detection_requires_door_or_building_eligibility() {
    use crate::element::{Camp, Entity};

    for observer_inside in [true, false] {
        let mut engine = EngineInner::new();
        // Slot 0 has Original creation order 31; frame 1 opens the common
        // modulo-16 blip cadence.
        engine.control.frame_counter = 1;
        let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
        let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
        install_test_building_sector(&mut engine, 42);

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("blipped inactive observer exists")
        else {
            panic!("blipped inactive observer changed kind")
        };
        observer.element.active = true;
        observer.element.blipped = true;
        observer.npc.life_points = 100;
        observer
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
        observer.element.set_position_map(MapPoint::new(0.0, 0.0));
        if observer_inside {
            observer.element.set_sector(Some(
                crate::position_interface::SectorHandle::new(42).unwrap(),
            ));
        }

        let Entity::Pc(pc) = engine
            .get_entity_mut(pc_id)
            .expect("blip-viewing PC exists")
        else {
            panic!("blip-viewing PC changed kind")
        };
        pc.element.active = true;
        pc.pc.playable = true;
        pc.pc.life_points = 100;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(20.0, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(20.0, 0.0));

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        engine
            .get_entity_mut(observer_id)
            .expect("blipped observer exists after fixture")
            .element_data_mut()
            .active = false;

        crate::sim_rng::with_seed(0xA013_B11F, |sim| engine.tick_enemy_ai(sim, &assets));

        assert_eq!(
            engine
                .get_entity(observer_id)
                .expect("blipped observer survives tick")
                .element_data()
                .blipped,
            !observer_inside,
            "inactive building NPCs run blip detection; inactive outdoor NPCs do not"
        );
    }
}

#[test]
fn inactive_door_transit_viewer_runs_blip_and_hearing_then_skips_optics() {
    use crate::ai::{AiLockFlags, AiState, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};
    use crate::order::{Order, OrderType};
    use crate::position_interface::DoorHandle;
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    // Slot 0 has Original creation order 31; frame 17 makes modified
    // frame 48, opening both the three-frame hearing cadence and the
    // modulo-16 blip cadence.
    engine.control.frame_counter = 17;
    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let runner_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("door-transit observer exists")
    else {
        panic!("door-transit observer changed kind")
    };
    observer.element.active = true;
    observer.element.blipped = true;
    observer.npc.life_points = 100;
    observer
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    observer.element.set_position_map(MapPoint::new(0.0, 0.0));
    observer.element.set_direction_instantly(4);
    observer.npc.view_direction = [1.0, 0.0];
    observer.npc.view_radius = 300;
    observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    observer.npc.eye_status = crate::element::EyeStatus::Stare;

    let Entity::Pc(runner) = engine
        .get_entity_mut(runner_id)
        .expect("door-transit runner exists")
    else {
        panic!("door-transit runner changed kind")
    };
    runner.element.active = true;
    runner.pc.playable = true;
    runner.pc.life_points = 100;
    runner
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(55.0, 0.0, 0.0));
    runner.element.set_position_map(MapPoint::new(55.0, 0.0));

    let mut movement = SequenceElement::new_movement(
        1,
        crate::element::Command::Move,
        Some(runner_id),
        OrderType::RunningUpright,
    );
    movement
        .orders
        .push_back(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("door-transit observer exists after fixture")
    else {
        panic!("door-transit observer changed kind after fixture")
    };
    observer.element.active = false;
    observer
        .element
        .sprite
        .position_iface
        .set_door_for_test(DoorHandle::new(0).expect("valid door index"));
    observer.npc.detectable_lists[DetectableType::Enemy as usize] = vec![Detectable {
        element: Some(runner_id),
        detectable_type: DetectableType::Enemy,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    }];
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    observer.npc.maximal_detection_suspect = 777;
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("door-transit observer has enemy AI");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::BUSY;
    ai.base.max_visibility = 15;

    crate::sim_rng::with_seed(0xA013_D00F, |sim| engine.tick_enemy_ai(sim, &assets));

    let Entity::Soldier(observer) = engine
        .get_entity(observer_id)
        .expect("door-transit observer survives tick")
    else {
        panic!("door-transit observer changed kind during tick")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy()
        .expect("door-transit observer retains enemy AI");
    assert!(
        !observer.element.blipped,
        "door transit passes the blip gate"
    );
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![StimulusType::EventHear],
        "door transit passes acoustics but the sector-only optical gate rejects it"
    );
    assert!(observer.npc.detectable_lists[DetectableType::Enemy as usize][0].heard_last_frame);
    assert!(!observer.npc.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame);
    assert_eq!(
        observer.npc.detection_suspects[DetectableType::Enemy as usize],
        999,
        "the door-only optical return must not scan or decay Enemy suspects"
    );
    assert_eq!(observer.npc.maximal_detection_suspect, 0);
    assert_eq!(ai.base.max_visibility, 0);
}

#[test]
fn royalist_blip_auto_reveal_obeys_the_common_sixteen_frame_cadence() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::element::{Camp, Entity};

    let mut engine = EngineInner::new();
    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("Royalist blip observer exists")
    else {
        panic!("Royalist blip observer changed kind")
    };
    observer.element.active = true;
    observer.element.blipped = true;
    observer.npc.life_points = 100;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    // Slot 0 has Original creation order 31. Frame 2 is closed; frame 1 below
    // is open for the common modulo-16 blip cadence.
    engine.control.frame_counter = 2;
    engine.tick_enemy_ai(sim, &assets);
    assert!(
        engine
            .get_entity(observer_id)
            .expect("Royalist blip observer survives closed cadence")
            .element_data()
            .blipped
    );

    engine.control.frame_counter = 1;
    engine.tick_enemy_ai(sim, &assets);
    assert!(
        !engine
            .get_entity(observer_id)
            .expect("Royalist blip observer survives open cadence")
            .element_data()
            .blipped
    );
}

#[test]
fn retained_detection_view_rebuilds_the_live_enemy_scan_on_replay() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::ai::{AiLockFlags, AiState, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};

    let mut engine = EngineInner::new();
    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let rising_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let already_seen_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("queued replay observer exists")
    else {
        panic!("queued replay observer changed kind")
    };
    observer.element.active = true;
    observer
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    observer.element.set_position_map(MapPoint::new(0.0, 0.0));
    observer.element.set_direction_instantly(4);
    observer.npc.life_points = 100;
    observer.npc.view_direction = [1.0, 0.0];
    observer.npc.view_radius = 300;
    observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    observer.npc.eye_status = crate::element::EyeStatus::Stare;

    for (id, x) in [(rising_id, 80.0), (already_seen_id, 120.0)] {
        let Entity::Pc(pc) = engine.get_entity_mut(id).expect("queued replay PC exists") else {
            panic!("queued replay PC changed kind")
        };
        pc.element.active = true;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(x, 0.0));
        pc.pc.life_points = 100;
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("queued replay observer exists after fixture")
    else {
        panic!("queued replay observer changed kind after fixture")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("queued replay observer has enemy AI");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::BUSY;
    ai.list_them.clear();

    observer.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    for (target_id, seen_last_frame) in [(rising_id, false), (already_seen_id, true)] {
        observer.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            seen_last_frame,
            shadow_seen_last_frame: true,
            ..Detectable::default()
        });
    }

    crate::sim_rng::with_seed(0xA013_0B23, |sim| engine.tick_enemy_ai(sim, &assets));
    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("queued replay observer retains enemy AI");
    assert_eq!(ai.base.stimulus_queue.len(), 1);
    assert_eq!(ai.base.current_state, AiState::Default);

    engine
        .get_entity_mut(observer_id)
        .and_then(Entity::ai_controller_mut)
        .expect("queued replay observer retains controller")
        .locks_flag_field = AiLockFlags::empty();
    engine.tick_ai_queued_stimuli(sim, &assets);

    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("queued replay observer retains enemy AI after replay");
    assert!(ai.base.stimulus_queue.is_empty());
    assert_eq!(ai.base.current_state, AiState::Attacking);
    assert_eq!(
        ai.base.primary_target,
        Some(crate::ai::AiEntityHandle::new(rising_id.index()))
    );
    assert_eq!(
        ai.list_them,
        vec![rising_id.index(), already_seen_id.index()],
        "retained VIEW replay must rebuild all currently latched enemies, not seed only its payload"
    );
    assert!(
        engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("queued replay observer remains an NPC after replay")
            .alerted,
        "accepted retained VIEW must set the persistent alert marker at dispatch time"
    );
}

#[test]
fn npc_out_of_view_precedes_same_slot_body_fifo() {
    use crate::ai::{AiState, Position, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};

    let mut engine = EngineInner::new();
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: ElementData {
            kind: ElementKind::Target,
            ..ElementData::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));

    let soldier_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let lost_pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let body_id = engine.add_entity(make_test_pc(crate::element::Posture::Dead));

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("out-of-view soldier exists")
    else {
        panic!("out-of-view observer changed kind")
    };
    soldier.element.active = true;
    soldier
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    soldier.element.set_position_map(MapPoint::new(0.0, 0.0));
    soldier.element.set_direction_instantly(4);
    soldier.npc.life_points = 100;
    soldier.npc.view_direction = [1.0, 0.0];
    soldier.npc.view_radius = 135;
    soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    soldier.npc.eye_status = crate::element::EyeStatus::Stare;
    let ai = soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("out-of-view soldier has enemy AI");
    ai.base.me = soldier_id.index();
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingObserve;
    ai.base.primary_target = Some(crate::ai::AiEntityHandle::new(lost_pc_id.index()));
    ai.base.seek_position = Position {
        x: -200.0,
        y: 0.0,
        ..Position::default()
    };
    ai.current_task_priority = task_priority::ENEMY;

    let Entity::Pc(lost_pc) = engine.get_entity_mut(lost_pc_id).expect("lost PC exists") else {
        panic!("lost target changed kind")
    };
    lost_pc.element.active = true;
    lost_pc
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(-200.0, 0.0, 0.0));
    lost_pc.element.set_position_map(MapPoint::new(-200.0, 0.0));
    lost_pc.pc.life_points = 100;

    let Entity::Pc(body) = engine.get_entity_mut(body_id).expect("body PC exists") else {
        panic!("body target changed kind")
    };
    body.element.active = true;
    body.element
        .set_position(crate::coordinates::WorldPoint3D::new(80.0, 0.0, 0.0));
    body.element.set_position_map(MapPoint::new(80.0, 0.0));
    body.pc.life_points = 0;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the living PC profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("out-of-view soldier exists before detection")
    else {
        panic!("out-of-view soldier changed kind")
    };
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    soldier.npc.detectable_lists[DetectableType::Body as usize].clear();
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(lost_pc_id),
        detectable_type: DetectableType::Enemy,
        seen_last_frame: true,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });
    soldier.npc.detectable_lists[DetectableType::Body as usize].push(Detectable {
        element: Some(body_id),
        detectable_type: DetectableType::Body,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0x0A01_30A7, |sim| engine.tick_enemy_ai(sim, &assets));

    let soldier = engine
        .get_entity(soldier_id)
        .and_then(Entity::npc_data)
        .expect("out-of-view soldier remains an NPC");
    assert!(
        !soldier.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame,
        "lost enemy must clear its seen latch"
    );
    assert!(
        soldier.detectable_lists[DetectableType::Body as usize].is_empty(),
        "visible body must commit and leave its one-shot detectable list"
    );
    let ai = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("out-of-view soldier retains enemy AI");
    assert_eq!(
        ai.base.detected_body,
        Some(crate::ai::AiEntityHandle::new(body_id.index())),
        "OUTOFVIEW must enter Seeking before the later BODY stimulus is handled"
    );
    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(ai.base.current_substate, Substate::SeekingBodyReactiontime);
}

#[test]
fn npc_detection_queues_every_rising_enemy_in_detectable_order() {
    use crate::ai::{AiState, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};

    fn observe(far_first: bool) -> (Vec<u32>, Vec<bool>, Vec<u32>) {
        let mut engine = EngineInner::new();
        engine.add_entity(Entity::Target(crate::element::ElementTarget {
            element: ElementData {
                kind: ElementKind::Target,
                ..ElementData::default()
            },
            fx: Default::default(),
            target: Default::default(),
        }));

        let soldier_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
        // A learned-through disguise remains in the Enemy bucket and must
        // emit ordinary EVENT_VIEW, never EVENT_SEES_BEGGAR.
        let far_pc_id = engine.add_entity(make_test_pc(crate::element::Posture::SimulatingBeggar));
        let near_pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

        let Entity::Soldier(soldier) = engine
            .get_entity_mut(soldier_id)
            .expect("multi-view soldier exists")
        else {
            panic!("multi-view observer changed kind")
        };
        soldier.element.active = true;
        soldier
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
        soldier.element.set_position_map(MapPoint::new(0.0, 0.0));
        soldier.element.set_direction_instantly(4);
        soldier.npc.life_points = 100;
        soldier.npc.view_direction = [1.0, 0.0];
        soldier.npc.view_radius = 300;
        soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
        soldier.npc.eye_status = crate::element::EyeStatus::Stare;

        for (pc_id, x) in [(far_pc_id, 120.0), (near_pc_id, 80.0)] {
            let Entity::Pc(pc) = engine.get_entity_mut(pc_id).expect("multi-view PC exists") else {
                panic!("multi-view target changed kind")
            };
            pc.element.active = true;
            pc.element
                .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
            pc.element.set_position_map(MapPoint::new(x, 0.0));
            pc.pc.life_points = 100;
        }

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .get_mut(0)
            .expect("fixture installs the PC character profile");
        profile.detection_speed_in_city = 100;
        profile.detection_speed_in_forest = 100;

        let Entity::Soldier(soldier) = engine
            .get_entity_mut(soldier_id)
            .expect("multi-view soldier exists before detection")
        else {
            panic!("multi-view observer changed kind")
        };
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("multi-view soldier has enemy AI");
        ai.base.me = soldier_id.index();
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultOnPost;
        ai.current_task_priority = task_priority::NONE;
        ai.base.got_the_beggar_trick = true;
        ai.list_them.clear();

        let ordered_targets = if far_first {
            [far_pc_id, near_pc_id]
        } else {
            [near_pc_id, far_pc_id]
        };
        let expected_order: Vec<u32> = ordered_targets.iter().map(|id| id.index()).collect();
        soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
        soldier.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
        for target_id in ordered_targets {
            soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
                element: Some(target_id),
                detectable_type: DetectableType::Enemy,
                shadow_seen_last_frame: true,
                ..Detectable::default()
            });
        }

        crate::sim_rng::with_seed(0xA013_0B1E, |sim| engine.tick_enemy_ai(sim, &assets));

        let soldier = engine
            .get_entity(soldier_id)
            .and_then(Entity::npc_data)
            .expect("multi-view soldier remains an NPC");
        let latches = soldier.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .map(|det| det.seen_last_frame)
            .collect();
        assert_eq!(
            soldier.follow_target,
            Some(ordered_targets[0]),
            "the first accepted VIEW must retain focus after the later FIFO entry"
        );
        let ai = engine
            .get_entity(soldier_id)
            .and_then(Entity::enemy_ai)
            .expect("multi-view soldier retains enemy AI");
        assert_eq!(ai.base.current_state, AiState::Attacking);
        assert_eq!(ai.base.current_substate, Substate::AttackingReactiontime);
        assert_eq!(
            ai.base.primary_target,
            Some(crate::ai::AiEntityHandle::new(expected_order[0])),
            "the first detectable's VIEW must win even when a later target is nearer"
        );
        assert_eq!(
            ai.base.last_stimulus_actor,
            Some(crate::ai::AiEntityHandle::new(expected_order[1])),
            "the second VIEW must run through its own complete Think boundary"
        );
        (ai.list_them.clone(), latches, expected_order)
    }

    let (far_then_near, far_then_near_latches, far_then_near_expected) = observe(true);
    let (near_then_far, near_then_far_latches, near_then_far_expected) = observe(false);

    assert_eq!(far_then_near_latches, vec![true, true]);
    assert_eq!(near_then_far_latches, vec![true, true]);
    assert_eq!(far_then_near, far_then_near_expected);
    assert_eq!(near_then_far, near_then_far_expected);
}

#[test]
fn npc_detection_view_rebinds_combat_data_to_the_queued_target() {
    use crate::ai::{AiState, Decision, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};

    let mut engine = EngineInner::new();
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: ElementData {
            kind: ElementKind::Target,
            ..ElementData::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let soldier_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let old_target_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let viewed_target_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("target-rebind soldier exists")
    else {
        panic!("target-rebind observer changed kind")
    };
    soldier.element.active = true;
    soldier
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    soldier.element.set_position_map(MapPoint::new(0.0, 0.0));
    soldier.element.set_direction_instantly(4);
    soldier.npc.life_points = 100;
    soldier.npc.view_direction = [1.0, 0.0];
    soldier.npc.view_radius = 300;
    soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    soldier.npc.eye_status = crate::element::EyeStatus::Stare;

    for (pc_id, x) in [(old_target_id, -200.0), (viewed_target_id, 5.0)] {
        let Entity::Pc(pc) = engine
            .get_entity_mut(pc_id)
            .expect("target-rebind PC exists")
        else {
            panic!("target-rebind target changed kind")
        };
        pc.element.active = true;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(x, 0.0));
        pc.pc.life_points = 100;
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the target-rebind PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("target-rebind soldier exists before detection")
    else {
        panic!("target-rebind observer changed kind")
    };
    let ai = soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("target-rebind soldier has enemy AI");
    ai.base.me = soldier_id.index();
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingJustWatching;
    ai.current_task_priority = task_priority::SEEKING;
    ai.base.primary_target = Some(crate::ai::AiEntityHandle::new(old_target_id.index()));
    ai.base.seek_position = crate::ai::Position {
        x: -200.0,
        y: 0.0,
        ..crate::ai::Position::default()
    };
    ai.forced_next_battle_decision = Decision::Fight;

    soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    soldier.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(viewed_target_id),
        detectable_type: DetectableType::Enemy,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0xA013_0B1F, |sim| engine.tick_enemy_ai(sim, &assets));

    let ai = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("target-rebind soldier retains enemy AI");
    // RHArtificialMalignity::BattleDecisions does not clear the forced
    // decision after using it. The serialized reset flag is never consulted.
    assert_eq!(
        (
            ai.base.primary_target,
            ai.base.last_stimulus_actor,
            ai.base.current_state,
            ai.base.current_substate,
            ai.forced_next_battle_decision,
        ),
        (
            Some(crate::ai::AiEntityHandle::new(viewed_target_id.index())),
            Some(crate::ai::AiEntityHandle::new(viewed_target_id.index())),
            AiState::Attacking,
            Substate::AttackingSwordfight,
            Decision::Fight,
        )
    );
    assert_eq!(ai.base.current_state, AiState::Attacking);
    assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
}

#[test]
fn royalist_detection_alert_does_not_bypass_strict_cadence() {
    use crate::ai::{AiState, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};

    fn observe(source_before_listener: bool) -> (bool, AiState, bool) {
        let mut engine = EngineInner::new();
        engine.add_entity(Entity::Target(crate::element::ElementTarget {
            element: ElementData {
                kind: ElementKind::Target,
                ..ElementData::default()
            },
            fx: Default::default(),
            target: Default::default(),
        }));

        let source = make_test_ai_soldier(Camp::Royalists);
        let listener = make_test_ai_soldier(Camp::Royalists);
        let (source_id, listener_id) = if source_before_listener {
            (engine.add_entity(source), engine.add_entity(listener))
        } else {
            let listener_id = engine.add_entity(listener);
            let source_id = engine.add_entity(source);
            (source_id, listener_id)
        };
        let target_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

        for (id, x) in [(source_id, 0.0), (listener_id, 20.0), (target_id, 80.0)] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("Royalist ordering soldier exists")
            else {
                panic!("Royalist ordering actor changed kind")
            };
            soldier.element.active = true;
            soldier
                .element
                .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
            soldier.element.set_position_map(MapPoint::new(x, 0.0));
            soldier.npc.life_points = 100;
            soldier.npc.view_radius = 200;
            soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
            if id == target_id {
                soldier.element.blipped = true;
            }
        }

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);

        for id in [source_id, listener_id] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("Royalist observer exists after fixture")
            else {
                panic!("Royalist observer changed kind after fixture")
            };
            let ai = soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("Royalist observer has enemy AI");
            ai.base.me = id.index();
            ai.base.current_state = AiState::Default;
            ai.base.current_substate = Substate::DefaultOnPost;
            ai.base.current_music_alert_status = crate::ai::AlertLevel::Green;
            ai.current_task_priority = task_priority::NONE;
            ai.base.primary_target = None;
            soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
            soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
                element: Some(target_id),
                detectable_type: DetectableType::Enemy,
                ..Detectable::default()
            });
        }

        let Entity::Soldier(source) = engine
            .get_entity_mut(source_id)
            .expect("source Royalist exists")
        else {
            panic!("source Royalist changed kind")
        };
        source.element.set_direction_instantly(4);
        source.npc.view_direction = [1.0, 0.0];
        source.npc.eye_status = crate::element::EyeStatus::Stare;

        let Entity::Soldier(listener) = engine
            .get_entity_mut(listener_id)
            .expect("listener Royalist exists")
        else {
            panic!("listener Royalist changed kind")
        };
        listener.element.set_direction_instantly(4);
        listener.npc.view_direction = [1.0, 0.0];
        listener.npc.eye_status = crate::element::EyeStatus::LookForward;

        const ORIGINAL_PRE_LEVEL_CREATIONS: u32 = 31;
        let source_creation_order = source_id.index() + ORIGINAL_PRE_LEVEL_CREATIONS;
        let listener_creation_order = listener_id.index() + ORIGINAL_PRE_LEVEL_CREATIONS;
        engine.control.frame_counter = (crate::ai_vision::DETECTION_FREQUENCY_ENEMY_NPC
            - source_creation_order % crate::ai_vision::DETECTION_FREQUENCY_ENEMY_NPC)
            % crate::ai_vision::DETECTION_FREQUENCY_ENEMY_NPC;
        assert!(
            (engine.control.frame_counter + source_creation_order)
                .is_multiple_of(crate::ai_vision::DETECTION_FREQUENCY_ENEMY_NPC),
            "source fixture must start on an open Royalist NPC detection gate"
        );
        assert!(
            !(engine.control.frame_counter + listener_creation_order)
                .is_multiple_of(crate::ai_vision::DETECTION_FREQUENCY_ENEMY_NPC),
            "listener fixture must start on a closed Royalist NPC detection gate"
        );
        crate::sim_rng::with_seed(0xA013_0B20, |sim| engine.tick_enemy_ai(sim, &assets));
        assert!(
            !engine
                .get_entity(target_id)
                .expect("Royalist target remains present")
                .element_data()
                .blipped,
            "Royalist HandleDetection must reveal its blipped NPC target at the detecting slot"
        );

        let source = engine
            .get_entity(source_id)
            .and_then(Entity::npc_data)
            .expect("source Royalist remains an NPC");
        let source_latch = source.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .find(|det| det.element == Some(target_id))
            .expect("source retains target detectable")
            .seen_last_frame;
        let source_ai = engine
            .get_entity(source_id)
            .and_then(Entity::enemy_ai)
            .expect("source Royalist retains enemy AI");
        assert_eq!(
            (source_latch, source_ai.base.current_state),
            (true, AiState::Attacking),
            "source Royalist must detect before its alert can test creation ordering"
        );

        let listener = engine
            .get_entity(listener_id)
            .and_then(Entity::npc_data)
            .expect("listener Royalist remains an NPC");
        let latch = listener.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .find(|det| det.element == Some(target_id))
            .expect("listener retains target detectable")
            .seen_last_frame;
        let ai = engine
            .get_entity(listener_id)
            .and_then(Entity::enemy_ai)
            .expect("listener Royalist retains enemy AI");
        (
            latch,
            ai.base.current_state,
            ai.base.primary_target == Some(crate::ai::AiEntityHandle::new(target_id.index())),
        )
    }

    let source_first = observe(true);
    assert_eq!(
        source_first,
        (false, AiState::Wondering, false),
        "a first Royalist's alert must not bypass the later Royalist's closed modulo-16 gate"
    );

    let listener_first = observe(false);
    assert_eq!(
        listener_first,
        (false, AiState::Wondering, false),
        "an earlier closed listener slot must not retroactively rescan after the source alerts it"
    );
}

#[test]
fn royalist_detection_retains_every_ordered_view_edge_while_ai_locked() {
    use crate::ai::{AiLockFlags, AiState, StimulusInfo, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};

    let mut engine = EngineInner::new();
    // Slot 0 has Original creation order 31; frame 1 opens its strict
    // modulo-16 Royalist NPC cadence.
    engine.control.frame_counter = 1;
    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let first_visible_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let lost_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let last_visible_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

    for (id, x) in [
        (observer_id, 0.0),
        (first_visible_id, 80.0),
        (lost_id, 100.0),
        (last_visible_id, 120.0),
    ] {
        let Entity::Soldier(soldier) = engine
            .get_entity_mut(id)
            .expect("Royalist multi-edge soldier exists")
        else {
            panic!("Royalist multi-edge actor changed kind")
        };
        soldier.element.active = true;
        soldier
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
        soldier.element.set_position_map(MapPoint::new(x, 0.0));
        soldier.npc.life_points = 100;
        soldier.element.blipped = id != observer_id;
    }

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("Royalist multi-edge observer exists")
    else {
        panic!("Royalist multi-edge observer changed kind")
    };
    observer.element.set_direction_instantly(4);
    observer.npc.view_direction = [1.0, 0.0];
    observer.npc.view_radius = 300;
    observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    observer.npc.eye_status = crate::element::EyeStatus::Stare;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Soldier(lost) = engine
        .get_entity_mut(lost_id)
        .expect("lost Royalist target exists after fixture")
    else {
        panic!("lost Royalist target changed kind after fixture")
    };
    // Original CleanUpDetectables removes dead enemies, not inactive living
    // ones. An inactive outdoor target remains in the list and emits the
    // falling OUTOFVIEW edge.
    lost.element.active = false;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("Royalist multi-edge observer exists after fixture")
    else {
        panic!("Royalist multi-edge observer changed kind after fixture")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("Royalist multi-edge observer has enemy AI");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.base.current_music_alert_status = crate::ai::AlertLevel::Green;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::BUSY;

    observer.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    for (target_id, seen_last_frame) in [
        (first_visible_id, false),
        (lost_id, true),
        (last_visible_id, false),
    ] {
        observer.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            seen_last_frame,
            ..Detectable::default()
        });
    }

    crate::sim_rng::with_seed(0xA013_0B21, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("Royalist multi-edge observer remains an NPC");
    assert_eq!(
        observer.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .map(|det| det.seen_last_frame)
            .collect::<Vec<_>>(),
        vec![true, false, true],
        "HandleDetection must settle every Royalist Enemy latch before Think"
    );
    assert!(
        !engine
            .get_entity(first_visible_id)
            .expect("first visible Royalist target remains present")
            .element_data()
            .blipped
            && !engine
                .get_entity(last_visible_id)
                .expect("last visible Royalist target remains present")
                .element_data()
                .blipped,
        "every rising Royalist Enemy edge must reveal its target before Think"
    );
    assert!(
        engine
            .get_entity(lost_id)
            .expect("lost Royalist target remains present")
            .element_data()
            .blipped,
        "a falling Royalist Enemy edge must not reveal its target"
    );
    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("Royalist multi-edge observer retains enemy AI");
    assert!(ai.base.outbox.detection.stimuli.is_empty());
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(
        ai.base.last_stimulus_actor,
        Some(crate::ai::AiEntityHandle::new(last_visible_id.index()))
    );
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .map(|stimulus| {
                let StimulusInfo::Human(target) = stimulus.info else {
                    panic!("Royalist Enemy edge lost its human payload")
                };
                (stimulus.stimulus_type, target)
            })
            .collect::<Vec<_>>(),
        vec![
            (
                StimulusType::EventView,
                crate::ai::AiEntityHandle::new(first_visible_id.index()),
            ),
            (
                StimulusType::EventOutOfView,
                crate::ai::AiEntityHandle::new(lost_id.index()),
            ),
            (
                StimulusType::EventView,
                crate::ai::AiEntityHandle::new(last_visible_id.index()),
            ),
        ],
        "Royalist HandleDetection must retain interleaved edges in detectable-list order"
    );
}

#[test]
fn royalist_enemy_cadence_stays_strict_when_staring_following_or_alerted() {
    use crate::ai::{AiLockFlags, AlertLevel};
    use crate::element::{Camp, Detectable, DetectableType, Entity, EyeStatus};

    for (eye_status, alert_status) in [
        (EyeStatus::Stare, AlertLevel::Green),
        (EyeStatus::Follow, AlertLevel::Green),
        (EyeStatus::LookForward, AlertLevel::Red),
    ] {
        let mut engine = EngineInner::new();
        let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
        let target_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

        for (id, x) in [(observer_id, 0.0), (target_id, 80.0)] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("strict-cadence soldier exists")
            else {
                panic!("strict-cadence actor changed kind")
            };
            soldier.element.active = true;
            soldier
                .element
                .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
            soldier.element.set_position_map(MapPoint::new(x, 0.0));
            soldier.npc.life_points = 100;
        }

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("strict-cadence observer exists after fixture")
        else {
            panic!("strict-cadence observer changed kind after fixture")
        };
        observer.element.set_direction_instantly(4);
        observer.npc.view_direction = [1.0, 0.0];
        observer.npc.view_radius = 200;
        observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
        observer.npc.eye_status = eye_status;
        let ai = observer
            .npc
            .ai_brain
            .enemy_mut()
            .expect("strict-cadence observer has EnemyAi");
        ai.base.current_music_alert_status = alert_status;
        ai.base.locks_flag_field = AiLockFlags::BUSY;
        observer.npc.detectable_lists[DetectableType::Enemy as usize] = vec![Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            seen_now: true,
            seen_last_frame: true,
            last_visibility: 0.25,
            ..Detectable::default()
        }];

        // Slot 0 has Original creation order 31; frame 2 is a closed strict
        // modulo-16 cadence frame.
        engine.control.frame_counter = 2;
        assert!(
            !(engine.control.frame_counter + observer_id.index() + 31)
                .is_multiple_of(crate::ai_vision::DETECTION_FREQUENCY_ENEMY_NPC)
        );
        crate::sim_rng::with_seed(0xA013_1600 + eye_status as u64, |sim| {
            engine.tick_enemy_ai(sim, &assets)
        });

        let observer = engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("strict-cadence observer retains NPC state");
        let detectable = &observer.detectable_lists[DetectableType::Enemy as usize][0];
        assert_eq!(
            (
                detectable.seen_now,
                detectable.seen_last_frame,
                detectable.last_visibility
            ),
            (true, true, 0.25),
            "Royalist {:?}/{:?} must reuse the cached sample without recomputing on a closed modulo-16 gate",
            eye_status,
            alert_status
        );
        assert!(
            observer
                .ai_brain
                .base()
                .expect("strict-cadence observer retains AI state")
                .stimulus_queue
                .is_empty()
        );
    }
}

#[test]
fn royalist_civilian_enemy_list_accepts_pc_but_not_lacklandist_soldier() {
    use crate::ai::{AiLockFlags, StimulusInfo, StimulusType};
    use crate::element::{AiBrain, Camp, DetectableType, Entity};

    let mut engine = EngineInner::new();
    // Slot 0 has Original creation order 31; frame 1 opens the Royalist
    // modulo-16 Enemy cadence.
    engine.control.frame_counter = 1;
    let civilian_id = engine.add_entity(make_test_civilian(crate::element::Posture::Upright));
    let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let lacklandist_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

    let Entity::Civilian(civilian) = engine
        .get_entity_mut(civilian_id)
        .expect("Royalist civilian exists")
    else {
        panic!("Royalist civilian changed kind")
    };
    civilian.element.active = true;
    civilian.civilian.cached_camp = Camp::Royalists;
    civilian.npc.life_points = 100;
    civilian.npc.ai_brain = AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(
        civilian_id.index(),
    )));
    civilian.element.set_direction_instantly(4);
    civilian.npc.view_direction = [1.0, 0.0];
    civilian.npc.view_radius = 200;
    civilian.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;

    for (id, x) in [(pc_id, 80.0), (lacklandist_id, 100.0)] {
        let entity = engine
            .get_entity_mut(id)
            .expect("Royalist-civilian target exists");
        entity.element_data_mut().active = true;
        entity
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(x, 0.0));
        match entity {
            Entity::Pc(pc) => pc.pc.life_points = 100,
            Entity::Soldier(soldier) => soldier.npc.life_points = 100,
            _ => panic!("Royalist-civilian target changed kind"),
        }
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the PC profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    // Enemy membership is established once by AI init's AddDetectable policy
    // (the tick preserves it, never rebuilds it): a Royalist civilian keeps
    // PCs and rejects soldiers of either camp.
    let init_detectables = crate::engine::ai::build_detectable_enemies_for(
        Camp::Royalists,
        true,
        civilian_id,
        &crate::engine::ai::build_potential_detectables(&engine),
    );
    assert_eq!(
        init_detectables
            .iter()
            .map(|detectable| detectable.element)
            .collect::<Vec<_>>(),
        vec![Some(pc_id)],
        "Original Royalist civilian AddDetectable accepts PCs only"
    );
    let civilian = engine
        .get_entity_mut(civilian_id)
        .and_then(Entity::npc_data_mut)
        .expect("Royalist civilian retains NPC state");
    civilian.detectable_lists[DetectableType::Enemy as usize] = init_detectables;
    civilian.detection_suspects[DetectableType::Enemy as usize] = 999;
    civilian
        .ai_brain
        .base_mut()
        .expect("Royalist civilian retains FriendlyAi")
        .locks_flag_field = AiLockFlags::BUSY;

    crate::sim_rng::with_seed(0xA013_C1A1, |sim| engine.tick_enemy_ai(sim, &assets));

    let civilian = engine
        .get_entity(civilian_id)
        .and_then(Entity::npc_data)
        .expect("Royalist civilian survives detection");
    assert_eq!(
        civilian.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .map(|detectable| detectable.element)
            .collect::<Vec<_>>(),
        vec![Some(pc_id)],
        "Original Royalist civilian AddDetectable accepts PCs only"
    );
    let ai = civilian
        .ai_brain
        .base()
        .expect("Royalist civilian retains FriendlyAi after detection");
    assert_eq!(
        ai.stimulus_queue
            .iter()
            .filter(|stimulus| stimulus.stimulus_type == StimulusType::EventView)
            .map(|stimulus| stimulus.info)
            .collect::<Vec<_>>(),
        vec![StimulusInfo::Human(crate::ai::AiEntityHandle::new(
            pc_id.index()
        ))]
    );
}

fn mixed_enemy_fifo_fixture(
    pc_first: bool,
) -> (EngineInner, LevelAssets, EntityId, EntityId, EntityId) {
    use crate::ai::{AiLockFlags, AiState, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};

    let mut engine = EngineInner::new();
    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let royalist_id = engine.add_entity(make_test_ai_soldier(Camp::Royalists));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("mixed-fifo observer exists")
    else {
        panic!("mixed-fifo observer changed kind")
    };
    observer.element.active = true;
    observer
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    observer.element.set_position_map(MapPoint::new(0.0, 0.0));
    observer.element.set_direction_instantly(4);
    observer.npc.life_points = 100;
    observer.npc.view_direction = [1.0, 0.0];
    observer.npc.view_radius = 300;
    observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    observer.npc.eye_status = crate::element::EyeStatus::Stare;

    let Entity::Pc(pc) = engine.get_entity_mut(pc_id).expect("mixed-fifo PC exists") else {
        panic!("mixed-fifo PC changed kind")
    };
    pc.element.active = true;
    pc.element
        .set_position(crate::coordinates::WorldPoint3D::new(80.0, 0.0, 0.0));
    pc.element.set_position_map(MapPoint::new(80.0, 0.0));
    pc.pc.life_points = 100;

    let Entity::Soldier(royalist) = engine
        .get_entity_mut(royalist_id)
        .expect("mixed-fifo Royalist target exists")
    else {
        panic!("mixed-fifo Royalist target changed kind")
    };
    royalist.element.active = true;
    royalist
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(120.0, 0.0, 0.0));
    royalist.element.set_position_map(MapPoint::new(120.0, 0.0));
    royalist.npc.life_points = 100;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("mixed-fifo observer exists after fixture")
    else {
        panic!("mixed-fifo observer changed kind after fixture")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("mixed-fifo observer has EnemyAi");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::BUSY;
    ai.base.got_the_beggar_trick = true;

    observer.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    let ordered = if pc_first {
        [pc_id, royalist_id]
    } else {
        [royalist_id, pc_id]
    };
    for target_id in ordered {
        observer.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            // Keep the oracle on VIEW order, not shadow predetection.
            shadow_seen_last_frame: true,
            ..Detectable::default()
        });
    }

    (engine, assets, observer_id, pc_id, royalist_id)
}

#[test]
fn enemy_outer_box_rejection_preserves_shadow_latch_but_entered_invisible_clears_it() {
    use crate::element::{DetectableType, Entity};

    fn shadow_latch_after_scan(target_x: f32) -> (bool, bool, f32) {
        let (mut engine, assets, observer_id, pc_id, _) = mixed_enemy_fifo_fixture(true);
        let Entity::Pc(pc) = engine
            .get_entity_mut(pc_id)
            .expect("outer-box target exists")
        else {
            panic!("outer-box target changed kind")
        };
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(target_x, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(target_x, 0.0));

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("outer-box observer exists")
        else {
            panic!("outer-box observer changed kind")
        };
        let enemies = &mut observer.npc.detectable_lists[DetectableType::Enemy as usize];
        enemies.retain(|detectable| detectable.element == Some(pc_id));
        let detectable = enemies
            .first_mut()
            .expect("outer-box fixture retains its PC detectable");
        detectable.shadow_seen_last_frame = true;
        detectable.seen_now = true;
        detectable.last_visibility = 0.0;
        observer.npc.detection_suspects[DetectableType::Enemy as usize] =
            crate::ai_vision::SHADOW_DETECTION_THRESHOLD as u16;

        crate::sim_rng::with_seed(0xA013_0B0E, |sim| engine.tick_enemy_ai(sim, &assets));

        let detectable = engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("outer-box observer retains NPC state")
            .detectable_lists[DetectableType::Enemy as usize]
            .first()
            .expect("outer-box observer retains PC detectable");
        (
            detectable.shadow_seen_last_frame,
            detectable.seen_now,
            detectable.last_visibility,
        )
    }

    let outside = shadow_latch_after_scan(400.0);
    assert_eq!(
        outside,
        (true, false, 0.0),
        "Original's outer else clears current visibility without calling HandlePredetection"
    );

    let entered_but_behind = shadow_latch_after_scan(-80.0);
    assert_eq!(
        entered_but_behind,
        (false, false, 0.0),
        "an entered target with zero sharpness must call HandlePredetection and clear the old latch"
    );
}

#[test]
fn lacklandist_mixed_pc_soldier_enemy_fifo_follows_detectable_order() {
    use crate::ai::{StimulusInfo, StimulusType};
    use crate::element::Entity;

    for pc_first in [true, false] {
        let (mut engine, assets, observer_id, pc_id, royalist_id) =
            mixed_enemy_fifo_fixture(pc_first);
        crate::sim_rng::with_seed(0xA013_F1F0, |sim| engine.tick_enemy_ai(sim, &assets));

        let ai = engine
            .get_entity(observer_id)
            .and_then(Entity::ai_controller)
            .expect("mixed-fifo observer retains its controller");
        let actual = ai
            .stimulus_queue
            .iter()
            .map(|stimulus| {
                assert_eq!(stimulus.stimulus_type, StimulusType::EventView);
                let StimulusInfo::Human(target) = stimulus.info else {
                    panic!("mixed Enemy VIEW lost its human target")
                };
                target
            })
            .collect::<Vec<_>>();
        let expected = if pc_first {
            vec![
                crate::ai::AiEntityHandle::new(pc_id.index()),
                crate::ai::AiEntityHandle::new(royalist_id.index()),
            ]
        } else {
            vec![
                crate::ai::AiEntityHandle::new(royalist_id.index()),
                crate::ai::AiEntityHandle::new(pc_id.index()),
            ]
        };
        assert_eq!(
            actual, expected,
            "one HandleDetection pass must retain interleaved PC/soldier insertion order"
        );
    }
}

#[test]
fn mixed_enemy_fifo_survives_detectable_mutation_between_entries() {
    use crate::ai::{AiLockFlags, StimulusInfo, StimulusType};
    use crate::element::{DetectableType, Entity};

    let (mut engine, assets, observer_id, pc_id, royalist_id) = mixed_enemy_fifo_fixture(true);
    crate::sim_rng::with_seed(0xA013_F1F1, |sim| {
        engine.tick_enemy_ai(sim, &assets);

        // Consume the first retained entry, then model the detectable-list
        // mutation that Original explicitly postpones until after its full
        // HandleDetection FIFO has been built. The second queued VIEW must be
        // independent of the now-live list.
        let first = engine
            .get_entity_mut(observer_id)
            .and_then(Entity::ai_controller_mut)
            .expect("mixed-fifo observer retains its controller")
            .stimulus_queue
            .remove(0);
        assert_eq!(first.stimulus_type, StimulusType::EventView);
        assert_eq!(
            first.info,
            StimulusInfo::Human(crate::ai::AiEntityHandle::new(pc_id.index()))
        );

        let observer = engine
            .get_entity_mut(observer_id)
            .and_then(Entity::npc_data_mut)
            .expect("mixed-fifo observer retains NPC state");
        observer.detectable_lists[DetectableType::Enemy as usize]
            .retain(|detectable| detectable.element != Some(royalist_id));
        observer
            .ai_brain
            .base_mut()
            .expect("mixed-fifo observer retains AI state")
            .locks_flag_field = AiLockFlags::empty();

        engine.tick_ai_queued_stimuli(sim, &assets);
    });

    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("mixed-fifo observer retains EnemyAi after replay");
    assert_eq!(
        ai.base.last_stimulus_actor,
        Some(crate::ai::AiEntityHandle::new(royalist_id.index())),
        "later mixed VIEW must already be queued before an earlier Think can mutate detectables"
    );
}

#[test]
#[should_panic(expected = "Enemy detectable target 999999 for NPC 0 is missing")]
fn mixed_enemy_walk_rejects_missing_detectable_target_with_context() {
    use crate::element::{DetectableType, Entity};

    let (mut engine, assets, observer_id, _, _) = mixed_enemy_fifo_fixture(true);
    let observer = engine
        .get_entity_mut(observer_id)
        .and_then(Entity::npc_data_mut)
        .expect("missing-target observer retains NPC state");
    observer.detectable_lists[DetectableType::Enemy as usize][0].element =
        Some(EntityId::Soldier(crate::entity_id::SoldierId(999_999)));

    crate::sim_rng::with_seed(0xA013_BAD1, |sim| engine.tick_enemy_ai(sim, &assets));
}

#[test]
#[should_panic(expected = "eligible civilian NPC 0 has no FriendlyAi brain during detection")]
fn mixed_enemy_walk_rejects_missing_observer_ai_with_context() {
    use crate::element::{Camp, Detectable, DetectableType, Entity};

    let mut engine = EngineInner::new();
    let civilian_id = engine.add_entity(make_test_civilian(crate::element::Posture::Upright));
    let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let Entity::Civilian(civilian) = engine
        .get_entity_mut(civilian_id)
        .expect("missing-AI civilian exists")
    else {
        panic!("missing-AI observer changed kind")
    };
    civilian.element.active = true;
    civilian.civilian.cached_camp = Camp::Lacklandists;
    civilian.npc.life_points = 100;
    civilian.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(pc_id),
        detectable_type: DetectableType::Enemy,
        ..Detectable::default()
    });

    let Entity::Pc(pc) = engine
        .get_entity_mut(pc_id)
        .expect("missing-AI target exists")
    else {
        panic!("missing-AI target changed kind")
    };
    pc.element.active = true;
    pc.pc.life_points = 100;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let Entity::Civilian(civilian) = engine
        .get_entity_mut(civilian_id)
        .expect("missing-AI civilian exists after fixture completion")
    else {
        panic!("missing-AI observer changed kind after fixture completion")
    };
    civilian.npc.ai_brain = crate::element::AiBrain::None;
    crate::sim_rng::with_seed(0xA013_BAD2, |sim| engine.tick_enemy_ai(sim, &assets));
}

#[test]
#[should_panic(expected = "eligible soldier NPC 0 has no EnemyAi brain during detection")]
fn mixed_enemy_walk_rejects_friendly_ai_on_a_soldier() {
    use crate::element::{AiBrain, Camp, Entity};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("wrong-AI soldier exists")
    else {
        panic!("wrong-AI observer changed kind")
    };
    soldier.element.active = true;
    soldier.npc.life_points = 100;
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("wrong-AI soldier survives fixture setup")
    else {
        panic!("wrong-AI observer changed kind after fixture")
    };
    soldier.npc.ai_brain = AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(
        soldier_id.index(),
    )));
    let _ = engine.enemy_optical_viewer_context_for_test(soldier_id);
}

#[test]
#[should_panic(expected = "eligible autonomous PC 0 has no EnemyAi brain during detection")]
fn autonomous_pc_detection_rejects_friendly_ai_brain() {
    use crate::element::{AiActorData, AiBrain, Entity};

    let mut engine = EngineInner::new();
    let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let Entity::Pc(pc) = engine
        .get_entity_mut(pc_id)
        .expect("wrong-AI autonomous PC exists")
    else {
        panic!("wrong-AI autonomous PC changed kind")
    };
    pc.element.active = true;
    pc.pc.life_points = 100;
    pc.pc.ai = Some(Box::new(AiActorData {
        ai_brain: AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(pc_id.index()))),
        ..AiActorData::default()
    }));

    let _ = engine.enemy_optical_viewer_context_for_test(pc_id);
}

#[test]
fn mixed_enemy_cleanup_removes_negative_life_targets() {
    use crate::element::{DetectableType, Entity};

    let (mut engine, assets, observer_id, pc_id, royalist_id) = mixed_enemy_fifo_fixture(true);
    let Entity::Pc(pc) = engine
        .get_entity_mut(pc_id)
        .expect("negative-life PC target exists")
    else {
        panic!("negative-life PC target changed kind")
    };
    pc.pc.life_points = -5;
    let Entity::Soldier(royalist) = engine
        .get_entity_mut(royalist_id)
        .expect("negative-life soldier target exists")
    else {
        panic!("negative-life soldier target changed kind")
    };
    royalist.npc.life_points = -7;

    crate::sim_rng::with_seed(0xA013_DEAD, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("negative-life observer retains NPC state");
    assert!(
        observer.detectable_lists[DetectableType::Enemy as usize].is_empty(),
        "CleanUpDetectables must use IsDead (life <= 0) for PCs and soldiers"
    );
}

#[test]
fn lacklandist_mixed_enemy_cadence_is_selected_per_entry() {
    use crate::ai::{AlertLevel, StimulusInfo, StimulusType};
    use crate::element::Entity;

    fn observed_targets(frame: u32) -> Vec<crate::ai::AiEntityHandle> {
        let (mut engine, assets, observer_id, pc_id, royalist_id) = mixed_enemy_fifo_fixture(true);
        engine.control.frame_counter = frame;
        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("cadence observer exists")
        else {
            panic!("cadence observer changed kind")
        };
        observer.npc.eye_status = crate::element::EyeStatus::LookForward;
        observer
            .npc
            .ai_brain
            .base_mut()
            .expect("cadence observer retains AI state")
            .current_music_alert_status = AlertLevel::Green;

        crate::sim_rng::with_seed(0xA013_CADE, |sim| engine.tick_enemy_ai(sim, &assets));

        let ai = engine
            .get_entity(observer_id)
            .and_then(Entity::ai_controller)
            .expect("cadence observer retains controller");
        let targets = ai
            .stimulus_queue
            .iter()
            .filter(|&stimulus| stimulus.stimulus_type == StimulusType::EventView)
            .map(|stimulus| {
                let StimulusInfo::Human(target) = stimulus.info else {
                    panic!("cadence VIEW lost its human target")
                };
                target
            })
            .collect::<Vec<_>>();
        let expected = if frame == 3 {
            vec![crate::ai::AiEntityHandle::new(pc_id.index())]
        } else {
            vec![
                crate::ai::AiEntityHandle::new(pc_id.index()),
                crate::ai::AiEntityHandle::new(royalist_id.index()),
            ]
        };
        assert_eq!(targets, expected);
        targets
    }

    // Observer slot 0 has Original creation order 31. Frame 3 opens only
    // the modulo-2 PC cadence; frame 1 opens both modulo-2 and modulo-16.
    assert_eq!(observed_targets(3).len(), 1);
    assert_eq!(observed_targets(1).len(), 2);
}

#[test]
fn closed_cadence_cannot_reuse_visibility_blocked_by_eyes_blip_or_guard() {
    use crate::ai::{AlertLevel, StimulusInfo, StimulusType};
    use crate::element::{DetectableType, Entity, EyeStatus};

    #[derive(Clone, Copy, Debug)]
    enum Blocker {
        BlindEyes,
        BlippedViewer,
        GuardedPc,
    }

    for blocker in [
        Blocker::BlindEyes,
        Blocker::BlippedViewer,
        Blocker::GuardedPc,
    ] {
        let (mut engine, assets, observer_id, pc_id, _) = mixed_enemy_fifo_fixture(true);
        // Modified frame 33 keeps both the PC cadence and the common blip
        // cadence closed.
        engine.control.frame_counter = 2;

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("closed-cadence observer exists")
        else {
            panic!("closed-cadence observer changed kind")
        };
        observer.npc.eye_status = if matches!(blocker, Blocker::BlindEyes) {
            EyeStatus::Closed
        } else {
            EyeStatus::LookForward
        };
        observer.element.blipped = matches!(blocker, Blocker::BlippedViewer);
        observer
            .npc
            .ai_brain
            .base_mut()
            .expect("closed-cadence observer retains AI state")
            .current_music_alert_status = AlertLevel::Green;
        let detectable = observer.npc.detectable_lists[DetectableType::Enemy as usize]
            .iter_mut()
            .find(|detectable| detectable.element == Some(pc_id))
            .expect("closed-cadence observer tracks PC");
        detectable.last_visibility = 1.0;
        detectable.seen_now = true;
        detectable.seen_last_frame = !matches!(blocker, Blocker::GuardedPc);

        if matches!(blocker, Blocker::GuardedPc) {
            let Entity::Pc(pc) = engine
                .get_entity_mut(pc_id)
                .expect("closed-cadence guarded PC exists")
            else {
                panic!("closed-cadence guarded target changed kind")
            };
            pc.pc.guard = Some(observer_id);
        }

        assert!(
            !(engine.control.frame_counter + observer_id.index() + 31)
                .is_multiple_of(crate::ai_vision::DETECTION_FREQUENCY_ENEMY_PC)
        );
        crate::sim_rng::with_seed(0xA013_1A00 + blocker as u64, |sim| {
            engine.tick_enemy_ai(sim, &assets)
        });

        let observer = engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("closed-cadence observer retains NPC state");
        let detectable = observer.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .find(|detectable| detectable.element == Some(pc_id))
            .expect("blocked PC remains detectable");
        assert_eq!(
            (
                detectable.seen_now,
                detectable.seen_last_frame,
                detectable.last_visibility,
            ),
            (false, false, 0.0),
            "{blocker:?} must invalidate cached visibility before cadence"
        );
        let ai = observer
            .ai_brain
            .base()
            .expect("closed-cadence observer retains AI state");
        let out_of_view_targets = ai
            .stimulus_queue
            .iter()
            .filter(|&stimulus| stimulus.stimulus_type == StimulusType::EventOutOfView)
            .map(|stimulus| {
                let StimulusInfo::Human(target) = stimulus.info else {
                    panic!("closed-cadence OUTOFVIEW lost its human target")
                };
                target
            })
            .collect::<Vec<_>>();
        let expected = if matches!(blocker, Blocker::GuardedPc) {
            Vec::new()
        } else {
            vec![crate::ai::AiEntityHandle::new(pc_id.index())]
        };
        assert_eq!(
            out_of_view_targets, expected,
            "{blocker:?} must preserve the Original falling-edge semantics"
        );
    }
}

#[test]
fn closed_cadence_cached_visibility_contributes_to_maximal_sharpness() {
    use crate::ai::AlertLevel;
    use crate::element::{DetectableType, Entity, EyeStatus};

    let (mut engine, assets, observer_id, pc_id, _) = mixed_enemy_fifo_fixture(true);
    // Observer slot 0 has Original creation order 31, so modified frame 33
    // closes the modulo-2 PC cadence.
    engine.control.frame_counter = 2;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("closed-cadence observer exists")
    else {
        panic!("closed-cadence observer changed kind")
    };
    observer
        .npc
        .ai_brain
        .base_mut()
        .expect("closed-cadence observer retains AI state")
        .view_alert_status = AlertLevel::Green;
    observer.npc.eye_status = EyeStatus::LookForward;
    for (kind, list) in observer.npc.detectable_lists.iter_mut().enumerate() {
        if kind == DetectableType::Enemy as usize {
            list.retain(|detectable| detectable.element == Some(pc_id));
        } else {
            list.clear();
        }
    }
    let detectable = observer.npc.detectable_lists[DetectableType::Enemy as usize]
        .first_mut()
        .expect("closed-cadence observer tracks PC");
    detectable.last_visibility = 1.0;
    detectable.seen_now = true;
    detectable.seen_last_frame = true;

    crate::sim_rng::with_seed(0xA013_1A10, |sim| engine.tick_enemy_ai(sim, &assets));

    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::ai_controller)
        .expect("closed-cadence observer retains AI state");
    assert_eq!(
        ai.max_visibility,
        u32::from(crate::ai_vision::BASE_VIEW_SPEED),
        "Original maximizes integer sharpness after cached visibility reuse"
    );
}

#[test]
fn persisted_lean_out_flag_controls_detection_sharpness_after_posture_changes() {
    use crate::ai::AlertLevel;
    use crate::element::{DetectableType, Entity, EyeStatus, Posture};

    let (mut engine, assets, observer_id, pc_id, _) = mixed_enemy_fifo_fixture(true);
    // Observer slot 0 has Original creation order 31, so modified frame 33
    // closes the modulo-2 PC cadence and reuses the exact cached visibility.
    engine.control.frame_counter = 2;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("lean-out observer exists")
    else {
        panic!("lean-out observer changed kind")
    };
    observer.element.posture = Posture::Upright;
    observer.npc.eye_status = EyeStatus::LookForward;
    // Original RefreshView clears bLeanOut only while replacing
    // EYES_LOOK_DOWNWARDS. If another path already selected LookForward, the
    // serialized flag remains true even though posture is now Upright.
    observer.npc.view_lean_out = true;
    observer
        .npc
        .ai_brain
        .base_mut()
        .expect("lean-out observer retains AI state")
        .view_alert_status = AlertLevel::Green;
    for (kind, list) in observer.npc.detectable_lists.iter_mut().enumerate() {
        if kind == DetectableType::Enemy as usize {
            list.retain(|detectable| detectable.element == Some(pc_id));
        } else {
            list.clear();
        }
    }
    let detectable = observer.npc.detectable_lists[DetectableType::Enemy as usize]
        .first_mut()
        .expect("lean-out observer tracks PC");
    detectable.last_visibility = 1.0;
    detectable.seen_now = true;
    detectable.seen_last_frame = true;

    crate::sim_rng::with_seed(0xA013_1A11, |sim| engine.tick_enemy_ai(sim, &assets));

    let Entity::Soldier(observer) = engine
        .get_entity(observer_id)
        .expect("lean-out observer remains present")
    else {
        panic!("lean-out observer changed kind")
    };
    assert_eq!(observer.element.posture, Posture::Upright);
    assert!(observer.npc.view_lean_out);
    assert_eq!(
        observer
            .npc
            .ai_brain
            .base()
            .expect("lean-out observer retains AI state")
            .max_visibility,
        u32::from(crate::ai_vision::LOOK_DOWN_BASE_VIEW_SPEED),
        "Original selects the sharpness multiplier from bLeanOut, not posture"
    );
}

#[test]
fn persisted_lean_out_flag_controls_non_enemy_detection_sharpness() {
    use crate::ai::AlertLevel;
    use crate::element::{Detectable, DetectableType, Entity, EyeStatus, Posture};

    let (mut engine, assets, observer_id, pc_id, _) = mixed_enemy_fifo_fixture(true);
    // Observer creation order 31 plus universal frame 2 produces modified
    // frame 33. Body's modulo-8 cadence is therefore closed, making the
    // persisted visibility sample the exact input to sharpness conversion.
    engine.control.frame_counter = 2;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("non-Enemy lean-out observer exists")
    else {
        panic!("non-Enemy lean-out observer changed kind")
    };
    observer.element.posture = Posture::Upright;
    observer.npc.eye_status = EyeStatus::LookForward;
    observer.npc.view_lean_out = true;
    observer
        .npc
        .ai_brain
        .base_mut()
        .expect("non-Enemy lean-out observer retains AI state")
        .view_alert_status = AlertLevel::Green;
    for list in &mut observer.npc.detectable_lists {
        list.clear();
    }
    observer.npc.detectable_lists[DetectableType::Body as usize].push(Detectable {
        element: Some(pc_id),
        detectable_type: DetectableType::Body,
        last_visibility: 1.0,
        seen_now: true,
        seen_last_frame: true,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0xA013_1A12, |sim| engine.tick_enemy_ai(sim, &assets));

    let Entity::Soldier(observer) = engine
        .get_entity(observer_id)
        .expect("non-Enemy lean-out observer remains present")
    else {
        panic!("non-Enemy lean-out observer changed kind")
    };
    assert_eq!(observer.element.posture, Posture::Upright);
    assert!(observer.npc.view_lean_out);
    assert_eq!(
        observer
            .npc
            .ai_brain
            .base()
            .expect("non-Enemy lean-out observer retains AI state")
            .max_visibility,
        u32::from(crate::ai_vision::LOOK_DOWN_BASE_VIEW_SPEED),
        "non-Enemy buckets use persisted bLeanOut for sharpness too"
    );
}

#[test]
fn blipped_lacklandist_in_door_transit_is_inside_for_the_pre_cadence_gate() {
    use crate::ai::{StimulusInfo, StimulusType};
    use crate::element::{DetectableType, Entity};
    use crate::position_interface::DoorHandle;

    let (mut engine, assets, observer_id, pc_id, royalist_id) = mixed_enemy_fifo_fixture(true);
    // Modified frame 34 keeps blip/NPC cadence closed while opening the
    // Lacklandist PC cadence.
    engine.control.frame_counter = 3;

    let Entity::Soldier(royalist) = engine
        .get_entity_mut(royalist_id)
        .expect("door-transit rear target exists")
    else {
        panic!("door-transit rear target changed kind")
    };
    royalist
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(-120.0, 0.0, 0.0));
    royalist
        .element
        .set_position_map(MapPoint::new(-120.0, 0.0));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("door-transit optical observer exists")
    else {
        panic!("door-transit optical observer changed kind")
    };
    observer.element.blipped = true;
    observer
        .element
        .sprite
        .position_iface
        .set_door_for_test(DoorHandle::new(0).expect("valid door index"));
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;

    assert!(
        !(engine.control.frame_counter + observer_id.index() + 31)
            .is_multiple_of(crate::ai_vision::DETECTION_FREQUENCY_ENEMY_NPC),
        "fixture must keep NPC blip auto-reveal closed"
    );
    crate::sim_rng::with_seed(0xA013_D016, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("door-transit optical observer retains NPC state");
    let view_targets = observer
        .ai_brain
        .base()
        .expect("door-transit optical observer retains AI state")
        .stimulus_queue
        .iter()
        .filter(|&stimulus| stimulus.stimulus_type == StimulusType::EventView)
        .map(|stimulus| {
            let StimulusInfo::Human(target) = stimulus.info else {
                panic!("door-transit VIEW lost its human target")
            };
            target
        })
        .collect::<Vec<_>>();
    assert_eq!(
        view_targets,
        vec![crate::ai::AiEntityHandle::new(pc_id.index())],
        "the door pointer passes the PC blip gate, but must not fabricate a same-building handle that reveals the rear soldier"
    );
}

#[test]
fn enemy_optics_reads_pc_order_from_live_creation_slot_state() {
    use crate::ai::{StimulusInfo, StimulusType};
    use crate::element::{DetectableType, Entity};
    use crate::order::OrderType;

    let (mut engine, assets, observer_id, pc_id, _) = mixed_enemy_fifo_fixture(true);
    let mut element = crate::sequence::SequenceElement::new_generic(
        1,
        crate::element::Command::Wait,
        Some(pc_id),
    );
    element.state = crate::sequence::SequenceState::InProgress;
    element.push_order(crate::order::Order::test_new(
        OrderType::SimulatingBeggar,
        0.0,
        0.0,
    ));
    let seq_id = engine.orders.sequence_manager.launch_element(element);
    let elem_idx = 0;

    let observer = engine
        .get_entity_mut(observer_id)
        .and_then(Entity::npc_data_mut)
        .expect("live-order observer retains NPC state");
    observer.detection_suspects[DetectableType::Enemy as usize] = 999;
    observer
        .ai_brain
        .base_mut()
        .expect("live-order observer retains AI state")
        .got_the_beggar_trick = false;

    crate::sim_rng::with_seed(0xA013_11E0, |sim| {
        engine.refresh_detection_after_world_snapshot_for_test(sim, &assets, |engine| {
            engine
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
                .expect("live-order sequence survives snapshot")
                .orders
                .front_mut()
                .expect("live-order sequence retains a front order")
                .order_type = OrderType::TransitionWaitingUprightSimulatingBeggar;
        });
    });

    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::ai_controller)
        .expect("live-order observer retains AI controller");
    assert!(
        ai.stimulus_queue.iter().any(|stimulus| {
            stimulus.stimulus_type == StimulusType::EventView
                && stimulus.info
                    == StimulusInfo::Human(crate::ai::AiEntityHandle::new(pc_id.index()))
        }),
        "the live beggar transition must replace the snapshotted resting disguise"
    );
    assert!(
        ai.got_the_beggar_trick,
        "seeing the live beggar transition must teach the observer the disguise trick"
    );
}

#[test]
fn enemy_optics_reads_pc_detection_z_from_live_creation_slot_posture() {
    use crate::element::{DetectableType, Entity, Posture};

    let (mut engine, assets, observer_id, pc_id, _) = mixed_enemy_fifo_fixture(true);
    let Entity::Pc(pc) = engine.get_entity_mut(pc_id).expect("live-Z PC exists") else {
        panic!("live-Z target changed kind")
    };
    pc.element
        .set_position(crate::coordinates::WorldPoint3D::new(15.0, 0.0, 20.0));
    pc.element
        .set_position_map_preserving_3d(MapPoint::new(15.0, -20.0));
    pc.element.posture = Posture::Upright;

    let observer = engine
        .get_entity_mut(observer_id)
        .and_then(Entity::npc_data_mut)
        .expect("live-Z observer retains NPC state");
    observer.detection_suspects[DetectableType::Enemy as usize] = 999;
    observer
        .ai_brain
        .base_mut()
        .expect("live-Z observer retains AI state")
        .got_the_beggar_trick = true;

    crate::sim_rng::with_seed(0xA013_11E1, |sim| {
        engine.refresh_detection_after_world_snapshot_for_test(sim, &assets, |engine| {
            let Entity::Pc(pc) = engine
                .get_entity_mut(pc_id)
                .expect("live-Z PC survives snapshot")
            else {
                panic!("live-Z target changed kind after snapshot")
            };
            pc.element.posture = Posture::Crouched;
        });
    });

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("live-Z observer retains NPC state");
    let detectable = observer.detectable_lists[DetectableType::Enemy as usize]
        .iter()
        .find(|detectable| detectable.element == Some(pc_id))
        .expect("live-Z observer retains the PC detectable");
    assert_eq!(
        detectable.last_visibility, 2.0,
        "live crouched detection Z must satisfy the 3D close-visibility gate"
    );
}

#[test]
fn lacklandist_enemy_optics_keeps_but_cannot_see_hollow_man() {
    use crate::ai::{StimulusInfo, StimulusType};
    use crate::element::{DetectableType, Entity};

    let (mut engine, assets, observer_id, pc_id, royalist_id) = mixed_enemy_fifo_fixture(true);
    engine
        .get_entity_mut(pc_id)
        .and_then(Entity::human_data_mut)
        .expect("hollow target retains human state")
        .hollow_man = true;

    crate::sim_rng::with_seed(0xA013_4011, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("hollow observer retains NPC state");
    assert_eq!(
        observer.detectable_lists[DetectableType::Enemy as usize].len(),
        2,
        "HollowMan is invisible, not cleaned up"
    );
    assert!(!observer.detectable_lists[DetectableType::Enemy as usize][0].seen_now);
    let ai = observer
        .ai_brain
        .base()
        .expect("hollow observer retains AI state");
    assert_eq!(ai.stimulus_queue.len(), 1);
    assert_eq!(ai.stimulus_queue[0].stimulus_type, StimulusType::EventView);
    assert_eq!(
        ai.stimulus_queue[0].info,
        StimulusInfo::Human(crate::ai::AiEntityHandle::new(royalist_id.index()))
    );
}

#[test]
fn civilian_enemy_optics_uses_the_common_npc_walk() {
    use crate::ai::{AiLockFlags, StimulusInfo, StimulusType};
    use crate::element::{Camp, Detectable, DetectableType, Entity};

    let mut engine = EngineInner::new();
    let civilian_id = engine.add_entity(make_test_civilian(crate::element::Posture::Upright));
    let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Civilian(civilian) = engine
        .get_entity_mut(civilian_id)
        .expect("optical civilian exists")
    else {
        panic!("optical civilian changed kind")
    };
    civilian.element.active = true;
    civilian.civilian.cached_camp = Camp::Lacklandists;
    civilian.npc.life_points = 100;
    civilian.npc.ai_brain = crate::element::AiBrain::Friendly(Box::new(
        crate::ai_friendly::FriendlyAi::new(civilian_id.index()),
    ));
    civilian.element.set_direction_instantly(4);
    civilian.npc.view_direction = [1.0, 0.0];
    civilian.npc.view_radius = 300;
    civilian.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    civilian.npc.eye_status = crate::element::EyeStatus::Stare;

    let Entity::Pc(pc) = engine
        .get_entity_mut(pc_id)
        .expect("civilian target exists")
    else {
        panic!("civilian target changed kind")
    };
    pc.element.active = true;
    pc.element
        .set_position(crate::coordinates::WorldPoint3D::new(80.0, 0.0, 0.0));
    pc.element.set_position_map(MapPoint::new(80.0, 0.0));
    pc.pc.life_points = 100;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Civilian(civilian) = engine
        .get_entity_mut(civilian_id)
        .expect("optical civilian exists after fixture")
    else {
        panic!("optical civilian changed kind after fixture")
    };
    let ai = civilian
        .npc
        .ai_brain
        .base_mut()
        .expect("optical civilian has FriendlyAi");
    ai.me = civilian_id.index();
    ai.locks_flag_field = AiLockFlags::BUSY;
    ai.got_the_beggar_trick = true;
    civilian.npc.detectable_lists[DetectableType::Enemy as usize] = vec![Detectable {
        element: Some(pc_id),
        detectable_type: DetectableType::Enemy,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    }];
    civilian.npc.detection_suspects[DetectableType::Enemy as usize] = 999;

    crate::sim_rng::with_seed(0xA013_C1A0, |sim| engine.tick_enemy_ai(sim, &assets));

    let ai = engine
        .get_entity(civilian_id)
        .and_then(Entity::ai_controller)
        .expect("optical civilian retains FriendlyAi");
    assert_eq!(ai.stimulus_queue.len(), 1);
    assert_eq!(ai.stimulus_queue[0].stimulus_type, StimulusType::EventView);
    assert_eq!(
        ai.stimulus_queue[0].info,
        StimulusInfo::Human(crate::ai::AiEntityHandle::new(pc_id.index()))
    );
    // The PC stands 80 units east of the civilian's eye point (beyond both
    // the very-close and halfcircle radii), so the committed sharpness is
    // BASE_VIEW_SPEED × DETECTION_FREQUENCY_ENEMY_PC × the distance curve,
    // truncated to an integer like the engine does.
    let expected_sharpness = (f32::from(crate::ai_vision::BASE_VIEW_SPEED)
        * crate::ai_vision::DETECTION_FREQUENCY_ENEMY_PC as f32
        * crate::ai_vision::distance_sharpness(80.0 * 80.0, 300.0))
        as u32;
    assert!(expected_sharpness > 0);
    assert_eq!(
        ai.max_visibility, expected_sharpness,
        "the shared NPC maximum must be published through FriendlyAi too"
    );
}

fn make_discovery_bonus(x: f32) -> Entity {
    let mut element = crate::element::ElementData {
        kind: crate::element::ElementKind::ObjectBonus,
        active: true,
        blipped: true,
        ..Default::default()
    };
    element.set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
    element.set_position_map(MapPoint::new(x, 0.0));
    Entity::Bonus(crate::element::ElementBonus {
        element,
        object: crate::element::ObjectData {
            object_type: crate::element::ObjectType::BonusApple,
            ..Default::default()
        },
    })
}

fn make_blipped_non_bonus(kind: crate::element::ElementKind) -> Entity {
    let mut element = crate::element::ElementData {
        kind,
        active: true,
        blipped: true,
        ..Default::default()
    };
    element.set_position(crate::coordinates::WorldPoint3D::new(10.0, 0.0, 0.0));
    element.set_position_map(MapPoint::new(10.0, 0.0));
    match kind {
        crate::element::ElementKind::ObjectScroll => {
            Entity::Scroll(crate::element::ElementScroll {
                element,
                object: crate::element::ObjectData {
                    object_type: crate::element::ObjectType::Scroll,
                    ..Default::default()
                },
                ..Default::default()
            })
        }
        crate::element::ElementKind::ObjectProjectile => {
            Entity::Projectile(crate::element::ElementProjectile {
                element,
                object: crate::element::ObjectData {
                    object_type: crate::element::ObjectType::Arrow,
                    ..Default::default()
                },
                projectile: Default::default(),
            })
        }
        crate::element::ElementKind::ObjectNet => Entity::Net(crate::element::ElementNet {
            element,
            object: crate::element::ObjectData {
                object_type: crate::element::ObjectType::Net,
                ..Default::default()
            },
            projectile: Default::default(),
            net: Default::default(),
        }),
        _ => panic!("unsupported non-bonus discovery fixture {kind:?}"),
    }
}

fn run_owner_envelopes(engine: &mut EngineInner, assets: &LevelAssets) {
    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for (id, entity) in engine.world.entities.occupied() {
        positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
    }
    crate::sim_rng::with_seed(0xB0A0_0013, |sim| {
        engine.tick_actor_owner_envelopes(sim, assets, &positions);
    });
}

#[test]
fn bonus_refresh_discovered_is_live_bonus_owned_freeze_safe_and_rng_free() {
    use crate::sim_rng::with_draw_trace;

    let mut engine = EngineInner::new();
    engine.ai.standard_view_polygon_radius = 100;
    let bonus_before = engine.add_entity(make_discovery_bonus(10.0));
    let hole = engine.add_entity(make_discovery_bonus(5_000.0));
    let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let bonus_after = engine.add_entity(make_discovery_bonus(10.0));
    let scroll = engine.add_entity(make_blipped_non_bonus(
        crate::element::ElementKind::ObjectScroll,
    ));
    let projectile = engine.add_entity(make_blipped_non_bonus(
        crate::element::ElementKind::ObjectProjectile,
    ));
    let net = engine.add_entity(make_blipped_non_bonus(
        crate::element::ElementKind::ObjectNet,
    ));
    engine.remove_entity(hole);
    let Entity::Pc(pc) = engine.get_entity_mut(pc_id).expect("discovery PC exists") else {
        panic!("discovery PC changed kind")
    };
    pc.element.active = true;
    pc.pc.life_points = 100;
    pc.element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    pc.element.set_position_map(MapPoint::new(0.0, 0.0));
    engine.set_actors_frozen(true);
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let (_, trace) = with_draw_trace(|| run_owner_envelopes(&mut engine, &assets));

    assert!(
        trace.is_empty(),
        "bonus discovery must not consume simulation RNG"
    );
    assert!(
        !engine
            .get_entity(bonus_before)
            .unwrap()
            .element_data()
            .blipped
    );
    assert!(
        !engine
            .get_entity(bonus_after)
            .unwrap()
            .element_data()
            .blipped
    );
    for id in [scroll, projectile, net] {
        assert!(
            engine.get_entity(id).unwrap().element_data().blipped,
            "only Entity::Bonus owns RefreshDiscovered; {id:?} was revealed"
        );
    }
}

#[test]
fn bonus_refresh_discovered_uses_live_pc_eligibility_and_original_shoulders_factor() {
    fn discovered(
        posture: crate::element::Posture,
        x: f32,
        active: bool,
        life: i16,
        unconscious: bool,
    ) -> bool {
        let mut engine = EngineInner::new();
        engine.ai.standard_view_polygon_radius = 100;
        let pc_id = engine.add_entity(make_test_pc(posture));
        let bonus_id = engine.add_entity(make_discovery_bonus(x));
        let Entity::Pc(pc) = engine.get_entity_mut(pc_id).unwrap() else {
            unreachable!()
        };
        pc.element.active = active;
        pc.pc.life_points = life;
        pc.human.unconscious = unconscious;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(0.0, 0.0));
        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let eye_z = engine
            .get_entity(pc_id)
            .unwrap()
            .compute_eyes_point(None)
            .unwrap()
            .z;
        engine
            .get_entity_mut(bonus_id)
            .unwrap()
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, eye_z));
        engine.refresh_bonus_discovered_for(&assets, bonus_id);
        !engine.get_entity(bonus_id).unwrap().element_data().blipped
    }

    assert!(!discovered(
        crate::element::Posture::Upright,
        105.0,
        true,
        100,
        false
    ));
    assert!(discovered(
        crate::element::Posture::OnShoulders,
        105.0,
        true,
        100,
        false
    ));
    assert!(!discovered(
        crate::element::Posture::Upright,
        10.0,
        false,
        100,
        false
    ));
    assert!(!discovered(
        crate::element::Posture::Upright,
        10.0,
        true,
        0,
        false
    ));
    assert!(!discovered(
        crate::element::Posture::Upright,
        10.0,
        true,
        100,
        true
    ));
}

#[test]
fn bonus_refresh_discovered_observes_owner_callback_order_and_spawned_later_slots() {
    fn observed(pc_first: bool) -> (bool, bool) {
        let mut engine = EngineInner::new();
        engine.ai.standard_view_polygon_radius = 100;
        let (pc_id, bonus_id) = if pc_first {
            let pc = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
            let bonus = engine.add_entity(make_discovery_bonus(10.0));
            (pc, bonus)
        } else {
            let bonus = engine.add_entity(make_discovery_bonus(10.0));
            let pc = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
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
        let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        for (id, entity) in engine.world.entities.occupied() {
            positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
        }
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
                    pc.element.posture = crate::element::Posture::OnShoulders;
                    pc.element
                        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
                    pc.element.set_position_map(MapPoint::new(0.0, 0.0));
                    spawned = Some(engine.add_entity(make_discovery_bonus(10.0)));
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

#[test]
fn entering_beggar_registers_only_intelligent_lacklandist_seekers() {
    use crate::ai::{AiState, Substate};
    use crate::element::{Camp, DetectableType, Posture};

    let mut engine = EngineInner::new();
    let beggar = engine.add_entity(make_test_pc(Posture::SimulatingBeggar));
    let eligible = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let low_iq = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let not_seeking = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let wrong_camp = engine.add_entity(make_test_ai_soldier(Camp::Royalists));

    for (id, iq, substate) in [
        // Hard difficulty doubles enemy IQ: the recorded boundary has a
        // base-IQ-15 seeker that Original admits at the effective threshold.
        (eligible, 15, Substate::SeekingSeekpointApproachingBeggar),
        (low_iq, 14, Substate::SeekingSeekpoint),
        (not_seeking, 100, Substate::DefaultOnPost),
        (wrong_camp, 100, Substate::SeekingSeekpoint),
    ] {
        let Entity::Soldier(soldier) = engine
            .get_entity_mut(id)
            .expect("test observer must remain present")
        else {
            panic!("test observer must remain a soldier");
        };
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test observer must retain enemy AI");
        ai.soldier_profile_iq = iq;
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = substate;
    }

    crate::engine::beggar::add_beggar_for_all_intelligent_seeking_soldiers(
        &mut engine.world.entities,
        &engine.mission_domain.diplomacy,
        beggar,
        crate::player_profile::DifficultyLevel::Hard,
    );
    // Original AddDetectable requires uniqueness; a repeated entry boundary
    // must not append the same beggar twice.
    crate::engine::beggar::add_beggar_for_all_intelligent_seeking_soldiers(
        &mut engine.world.entities,
        &engine.mission_domain.diplomacy,
        beggar,
        crate::player_profile::DifficultyLevel::Hard,
    );

    let beggar_idx = DetectableType::Beggar as usize;
    for (id, expected) in [
        (eligible, true),
        (low_iq, false),
        (not_seeking, false),
        (wrong_camp, false),
    ] {
        let list = &engine
            .get_entity(id)
            .and_then(Entity::npc_data)
            .expect("test observer must retain NPC data")
            .detectable_lists[beggar_idx];
        assert_eq!(list.len(), usize::from(expected));
        assert!(
            list.iter()
                .all(|detectable| detectable.element == Some(beggar)
                    && detectable.detectable_type == DetectableType::Beggar),
            "unexpected beggar registration for {id:?}"
        );
    }
}
