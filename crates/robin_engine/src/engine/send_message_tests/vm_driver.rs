use super::*;

fn animated_sprite() -> crate::sprite::Sprite {
    let script = crate::sprite_script::SpriteScript {
        action_id: 0,
        action_done: 2,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0, 0, 0],
    };
    let mut sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]),
    );
    sprite.current_row = 0;
    sprite.current_frame = 0;
    sprite.frame_count = 0;
    sprite
}

fn animated_scroll() -> Entity {
    Entity::Scroll(crate::element::ElementScroll {
        element: ElementData {
            kind: ElementKind::ObjectScroll,
            active: true,
            sprite: animated_sprite(),
            ..Default::default()
        },
        script_hourglass_timeout: 24,
        ..Default::default()
    })
}

fn animated_target(progression: crate::sprite::FrameProgression) -> Entity {
    Entity::Target(crate::element::ElementTarget {
        element: ElementData {
            kind: ElementKind::Target,
            active: true,
            sprite: animated_sprite(),
            ..Default::default()
        },
        fx: Default::default(),
        target: crate::element::TargetData {
            progression: progression as u32,
            ..Default::default()
        },
    })
}

fn animated_bonus(object_type: crate::element::ObjectType, active: bool) -> Entity {
    Entity::Bonus(crate::element::ElementBonus {
        element: ElementData {
            kind: ElementKind::ObjectBonus,
            active,
            sprite: animated_sprite(),
            ..Default::default()
        },
        object: crate::element::ObjectData {
            object_type,
            ..Default::default()
        },
    })
}

fn empty_positions(
    engine: &EngineInner,
) -> crate::entities::EntitySlots<Option<crate::coordinates::MapPoint>> {
    crate::entities::EntitySlots::filled(engine.world.entities.len(), None)
}

#[test]
fn due_scroll_self_deactivation_keeps_entry_active_animation_order() {
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(message_script());
    let scroll_id = engine.add_entity(animated_scroll());
    let handle = ScriptHandleCodec::actor_handle(scroll_id);
    let instance = engine
        .scripts
        .mission
        .as_ref()
        .unwrap()
        .manager
        .create_instance("SelfDeactivatingScroll")
        .expect("self-deactivating scroll class");
    engine
        .scripts
        .mission
        .as_mut()
        .unwrap()
        .scroll_instances
        .insert(handle, instance);
    let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.attach_script_bindings(&assets);

    engine.tick_actor_owner_envelopes(&crate::sim_rng::test_context(), &assets, &positions);

    let Entity::Scroll(scroll) = engine
        .get_entity(scroll_id)
        .expect("scroll survives callback")
    else {
        unreachable!()
    };
    assert!(!scroll.element.active, "due callback ran before animation");
    assert_eq!(
        scroll.script_hourglass_timeout, 0,
        "due timeout reset after VM"
    );
    assert_eq!(
        scroll.element.sprite.current_frame, 1,
        "entry-active Scroll still animates after self-deactivation"
    );

    let frozen_id = engine.add_entity(animated_scroll());
    let frozen_handle = ScriptHandleCodec::actor_handle(frozen_id);
    let frozen_instance = engine
        .scripts
        .mission
        .as_ref()
        .unwrap()
        .manager
        .create_instance("SelfDeactivatingScroll")
        .expect("frozen self-deactivating scroll class");
    engine
        .scripts
        .mission
        .as_mut()
        .unwrap()
        .scroll_instances
        .insert(frozen_handle, frozen_instance);
    engine.set_actors_frozen(true);
    let positions = empty_positions(&engine);
    engine.tick_actor_owner_envelopes(&crate::sim_rng::test_context(), &assets, &positions);
    let Entity::Scroll(frozen_scroll) = engine.get_entity(frozen_id).unwrap() else {
        unreachable!()
    };
    assert!(
        !frozen_scroll.element.active,
        "FrozenAll does not suppress Scroll VM"
    );
    assert_eq!(frozen_scroll.script_hourglass_timeout, 0);
    assert_eq!(
        frozen_scroll.element.sprite.current_frame, 0,
        "FrozenAll gates only the sprite step"
    );
}

#[test]
fn due_scroll_callback_changes_same_slot_freeze_gate_live() {
    fn run(class_name: &str, initially_frozen: bool) -> (bool, u16) {
        let mut engine = EngineInner::new();
        engine.scripts.mission = Some(message_script());
        let scroll_id = engine.add_entity(animated_scroll());
        let handle = ScriptHandleCodec::actor_handle(scroll_id);
        let instance = engine
            .scripts
            .mission
            .as_ref()
            .unwrap()
            .manager
            .create_instance(class_name)
            .unwrap_or_else(|error| panic!("missing test scroll class {class_name}: {error}"));
        engine
            .scripts
            .mission
            .as_mut()
            .unwrap()
            .scroll_instances
            .insert(handle, instance);
        engine.set_actors_frozen(initially_frozen);
        let positions = empty_positions(&engine);
        let assets = LevelAssets::new();
        engine.attach_script_bindings(&assets);

        engine.tick_actor_owner_envelopes(&crate::sim_rng::test_context(), &assets, &positions);

        (
            engine.actors_frozen(),
            engine
                .get_entity(scroll_id)
                .unwrap()
                .element_data()
                .sprite
                .current_frame,
        )
    }

    assert_eq!(
        run("FreezeOnScroll", false),
        (true, 0),
        "callback FreezeAll(true) suppresses this same-slot sprite step"
    );
    assert_eq!(
        run("FreezeOffScroll", true),
        (false, 1),
        "callback FreezeAll(false) permits this same-slot sprite step"
    );
}

#[test]
#[should_panic(expected = "disappeared immediately after live legacy-slot resolution")]
fn resolved_static_owner_must_still_exist_at_dispatch() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(animated_target(crate::sprite::FrameProgression::Default));
    engine.remove_entity(owner);
    engine.tick_static_entity_hourglass_for(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
    );
}

#[test]
fn target_bored_rng_draws_follow_live_slot_order_exactly_once() {
    fn run(seed: u64, reverse: bool) -> (u16, u16) {
        let mut engine = EngineInner::new();
        let (a, b) = if reverse {
            let b = engine.add_entity(animated_target(crate::sprite::FrameProgression::BoredAnim));
            let a = engine.add_entity(animated_target(crate::sprite::FrameProgression::BoredAnim));
            (a, b)
        } else {
            let a = engine.add_entity(animated_target(crate::sprite::FrameProgression::BoredAnim));
            let b = engine.add_entity(animated_target(crate::sprite::FrameProgression::BoredAnim));
            (a, b)
        };
        let positions = empty_positions(&engine);
        let assets = LevelAssets::new();
        crate::sim_rng::with_seed(seed, |sim| {
            engine.tick_actor_owner_envelopes(sim, &assets, &positions);
        });
        (
            engine
                .get_entity(a)
                .unwrap()
                .element_data()
                .sprite
                .current_frame,
            engine
                .get_entity(b)
                .unwrap()
                .element_data()
                .sprite
                .current_frame,
        )
    }

    let seed = (0_u64..1_000_000)
        .find(|seed| {
            crate::sim_rng::with_seed(*seed, |sim| {
                let first =
                    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::SpriteBoredStart, ..250);
                let second =
                    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::SpriteBoredStart, ..250);
                (first == 0) != (second == 0)
            })
        })
        .expect("reviewed RNG stream must expose distinct first/second BoredAnim draws");
    let forward = run(seed, false);
    assert_ne!(forward.0, forward.1);
    assert_eq!(run(seed, true), (forward.1, forward.0));
}

#[test]
fn concrete_static_objects_run_once_and_broad_objects_stay_in_their_lanes() {
    let mut engine = EngineInner::new();
    let target = engine.add_entity(animated_target(crate::sprite::FrameProgression::Default));
    let ale = engine.add_entity(animated_bonus(crate::element::ObjectType::Ale, false));
    let cape = engine.add_entity(animated_bonus(crate::element::ObjectType::Cape, false));
    let bonus = engine.add_entity(animated_bonus(
        crate::element::ObjectType::BonusApple,
        false,
    ));
    let projectile = engine.add_entity(Entity::Projectile(crate::element::ElementProjectile {
        element: ElementData {
            kind: ElementKind::ObjectProjectile,
            active: true,
            sprite: animated_sprite(),
            ..Default::default()
        },
        object: crate::element::ObjectData {
            object_type: crate::element::ObjectType::Wasp,
            ..Default::default()
        },
        projectile: Default::default(),
    }));
    let net = engine.add_entity(Entity::Net(crate::element::ElementNet {
        element: ElementData {
            kind: ElementKind::ObjectNet,
            active: true,
            sprite: animated_sprite(),
            ..Default::default()
        },
        object: crate::element::ObjectData {
            object_type: crate::element::ObjectType::Net,
            ..Default::default()
        },
        projectile: Default::default(),
        net: Default::default(),
    }));
    let assets = LevelAssets::new();
    let sim = crate::sim_rng::test_context();

    let projectile_frame = engine
        .get_entity(projectile)
        .unwrap()
        .element_data()
        .sprite
        .current_frame;
    let net_frame = engine
        .get_entity(net)
        .unwrap()
        .element_data()
        .sprite
        .current_frame;
    let positions = empty_positions(&engine);
    engine.tick_actor_owner_envelopes(&sim, &assets, &positions);

    assert!(
        engine.get_entity(ale).is_none(),
        "inactive RHElementAle returns false"
    );
    assert_eq!(
        engine
            .get_entity(cape)
            .unwrap()
            .element_data()
            .sprite
            .current_frame,
        1
    );
    assert_eq!(
        engine
            .get_entity(bonus)
            .unwrap()
            .element_data()
            .sprite
            .current_frame,
        1
    );
    assert_eq!(
        engine
            .get_entity(target)
            .unwrap()
            .element_data()
            .sprite
            .current_frame,
        1
    );
    assert_eq!(
        engine
            .get_entity(projectile)
            .unwrap()
            .element_data()
            .sprite
            .current_frame,
        projectile_frame
    );
    assert_eq!(
        engine
            .get_entity(net)
            .unwrap()
            .element_data()
            .sprite
            .current_frame,
        net_frame
    );

    let mobile_child = engine.add_entity(Entity::Fx(crate::element::ElementFx {
        element: ElementData {
            kind: ElementKind::Fx,
            active: true,
            sprite: animated_sprite(),
            ..Default::default()
        },
        fx: crate::element::FxData {
            mobile_index: Some(0),
            ..Default::default()
        },
    }));
    engine.tick_static_entity_hourglass_for(&sim, &assets, mobile_child);
    engine.tick_static_entity_hourglass_for(&sim, &assets, projectile);
    assert_eq!(
        engine
            .get_entity(mobile_child)
            .unwrap()
            .element_data()
            .sprite
            .current_frame,
        0
    );
    assert_eq!(
        engine
            .get_entity(projectile)
            .unwrap()
            .element_data()
            .sprite
            .current_frame,
        projectile_frame
    );
}

#[test]
fn completed_fx_patch_is_visible_to_the_later_live_slot() {
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(message_script());
    let fx = Entity::Fx(crate::element::ElementFx {
        element: ElementData {
            kind: ElementKind::Fx,
            active: true,
            sprite: animated_sprite(),
            ..Default::default()
        },
        fx: crate::element::FxData {
            patch_index: crate::patch::PatchIndex::new(0),
            ..Default::default()
        },
    });
    let fx_id = engine.add_entity(fx);
    let later = engine.add_entity(animated_target(crate::sprite::FrameProgression::Default));
    engine
        .script_domains
        .interactables
        .patches
        .push(crate::patch::Patch {
            active: true,
            in_transition: true,
            applied: false,
            ..Default::default()
        });
    let Entity::Fx(fx) = engine.get_entity_mut(fx_id).unwrap() else {
        unreachable!()
    };
    fx.element.sprite.current_frame = 1;
    let assets = LevelAssets::new();
    let mut later_observed_final = false;
    let sim = crate::sim_rng::test_context();

    engine.tick_actor_animation_action_change_slots_with_hooks(
        &sim,
        &assets,
        |engine, owner| {
            engine.tick_static_entity_hourglass_for(&sim, &assets, owner);
            if owner == later {
                let patch = &engine.script_domains.interactables.patches[0];
                later_observed_final = patch.applied && !patch.in_transition;
            }
        },
        |_, _| {},
        |_, _, _, _, _, _, _| {},
        |_, _| {},
    );
    assert!(
        later_observed_final,
        "later slot sees synchronous SetInTransition(false)+ApplyFinal"
    );
}

#[test]
fn ownerless_send_message_routes_to_global_process_message() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    engine
        .call_external_native(
            &crate::sim_rng::test_context(),
            &assets,
            "SendMessage",
            &[0, 2718],
        )
        .expect("ownerless SendMessage should use RHEngine::ProcessMessage");

    assert_eq!(
        engine
            .scripts
            .mission
            .as_ref()
            .expect("script installed")
            .state
            .globals
            .get(&902),
        Some(&2718)
    );
}

#[test]
fn every_script_vm_flavor_drives_yields_through_the_shared_engine_boundary() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let actor_id = engine.add_entity(scripted_soldier("YieldingFlavor"));
    let actor_handle = bind_script_actor(&mut engine, actor_id, "YieldingFlavor");
    let zone = 7;
    let target_handle = ScriptHandleCodec::actor_handle_from_index(7001);
    let scroll_handle = ScriptHandleCodec::actor_handle_from_index(7002);
    let path = crate::ai::PathId::new(9).unwrap();
    {
        let script = engine.scripts.mission.as_mut().unwrap();
        let zone_instance = script.manager.create_instance("YieldingFlavor").unwrap();
        let target_instance = script.manager.create_instance("YieldingFlavor").unwrap();
        let scroll_instance = script.manager.create_instance("YieldingFlavor").unwrap();
        let waypoint_instance = script.manager.create_instance("YieldingFlavor").unwrap();
        script.zone_instances.insert(zone, zone_instance);
        script
            .target_instances
            .insert(target_handle, target_instance);
        script
            .scroll_instances
            .insert(scroll_handle, scroll_instance);
        script
            .waypoint_instances
            .insert((path, 3), waypoint_instance);
    }

    let calls = [
        (
            super::ScriptVmKey::Global,
            "Hourglass",
            Vec::new(),
            crate::natives::ScriptCallFrame::default(),
            4240,
        ),
        (
            super::ScriptVmKey::Actor(actor_handle),
            "Initialize",
            Vec::new(),
            crate::natives::ScriptCallFrame::actor(actor_handle),
            4241,
        ),
        (
            super::ScriptVmKey::Zone(zone),
            "EnterZone",
            vec![actor_handle],
            crate::natives::ScriptCallFrame::default(),
            4241,
        ),
        (
            super::ScriptVmKey::Target(target_handle),
            "ActivatedByArrow",
            vec![actor_handle],
            crate::natives::ScriptCallFrame::actor(target_handle),
            4241,
        ),
        (
            super::ScriptVmKey::Scroll(scroll_handle),
            "IsTaken",
            vec![actor_handle],
            crate::natives::ScriptCallFrame::scroll(scroll_handle),
            4241,
        ),
        (
            super::ScriptVmKey::Waypoint(path, 3),
            "ReachPoint",
            vec![actor_handle],
            crate::natives::ScriptCallFrame::default(),
            4241,
        ),
    ];
    for (key, function, params, frame, expected_message) in calls {
        engine
            .scripts
            .mission
            .as_mut()
            .unwrap()
            .state
            .globals
            .insert(902, 0);
        engine
            .call_script_vm(
                &crate::sim_rng::test_context(),
                &assets,
                key,
                function,
                &params,
                frame,
            )
            .unwrap_or_else(|error| panic!("{key:?}.{function} failed: {error}"));
        assert_eq!(
            engine
                .scripts
                .mission
                .as_ref()
                .unwrap()
                .state
                .globals
                .get(&902),
            Some(&expected_message),
            "{key:?}.{function} did not synchronously resume after ownerless ProcessMessage"
        );
    }
}

#[test]
fn shared_driver_preserves_same_actor_outer_activation() {
    let (mut engine, _receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerSelf",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("A→A callback should resume its outer activation");

    let globals = &engine
        .scripts
        .mission
        .as_ref()
        .expect("script installed")
        .state
        .globals;
    assert_eq!(globals.get(&900), Some(&314), "nested callback completed");
    assert_eq!(
        globals.get(&901),
        Some(&2),
        "outer callback resumed after child"
    );
}

#[test]
fn self_reentrant_driver_preserves_and_shares_the_instance_member_heap() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let actor_id = engine.add_entity(scripted_soldier("HeapA"));
    let handle = bind_script_actor(&mut engine, actor_id, "HeapA");

    engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerSelf",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("A→A member-heap callback");

    let script = engine.scripts.mission.as_ref().unwrap();
    assert_eq!(
        script.state.globals.get(&905),
        Some(&20),
        "heap={}, globals={:?}",
        script_instance_heap_word(script, handle),
        script.state.globals
    );
    assert_eq!(script_instance_heap_word(script, handle), 3);
}

#[test]
fn completed_reentrant_continuation_round_trips_as_an_idle_snapshot() {
    let (mut engine, _receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerSelf",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("first self-reentrant callback");
    let program = engine
        .scripts
        .mission
        .as_ref()
        .expect("mission")
        .manager
        .program
        .clone();
    let json = serde_json::to_string(&engine).expect("idle engine snapshot");
    let mut restored: EngineInner = serde_json::from_str(&json).expect("restore idle snapshot");
    restored
        .scripts
        .mission
        .as_mut()
        .expect("restored mission")
        .attach_program(program);
    restored.attach_script_bindings(&assets);
    restored
        .scripts
        .mission
        .as_mut()
        .unwrap()
        .state
        .globals
        .insert(901, 0);

    restored
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerSelf",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("restored instance accepts another self-reentrant callback");
    assert_eq!(
        restored
            .scripts
            .mission
            .as_ref()
            .unwrap()
            .state
            .globals
            .get(&901),
        Some(&2)
    );
}

#[test]
fn real_active_driver_rejects_snapshot_and_idle_driver_serializes() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let actor_id = engine.add_entity(scripted_soldier("YieldingFlavor"));
    let handle = bind_script_actor(&mut engine, actor_id, "YieldingFlavor");

    super::script::arm_active_driver_snapshot_probe();
    engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(handle),
            "EmitEffect",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("effect-producing callback");
    let error = super::script::take_active_driver_snapshot_error()
        .expect("effect drain executed the active-driver snapshot probe");
    assert!(error.contains("active script callback"), "{error}");
    let script = engine.scripts.mission.as_ref().unwrap();
    assert_eq!(script.active_call_frame_count(), 0);
    serde_json::to_string(&engine).expect("idle full-engine state is snapshot-safe");
}

#[test]
fn shared_driver_preserves_a_b_a_activation_stack() {
    let (mut engine, _receiver, a_handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let relay_id = engine.add_entity(scripted_soldier("RelayReceiver"));
    let b_handle = ScriptHandleCodec::actor_handle(relay_id);
    let simulation = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &simulation,
        &mut engine.world.entities,
        &mut engine.ai.global,
        &mut engine.world.fast_grid,
    );
    assert!(
        engine
            .scripts
            .mission
            .as_mut()
            .expect("script installed")
            .bind_actor(
                b_handle,
                "RelayReceiver",
                &mut engine.script_domains,
                &capabilities,
            )
    );

    engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(a_handle),
            "TriggerRelay",
            &[b_handle],
            crate::natives::ScriptCallFrame::actor(a_handle),
        )
        .expect("A→B→A callback stack should fully unwind");

    let globals = &engine
        .scripts
        .mission
        .as_ref()
        .expect("script installed")
        .state
        .globals;
    assert_eq!(globals.get(&900), Some(&20), "B re-entered A");
    assert_eq!(globals.get(&903), Some(&3), "outer A resumed last");
}

#[test]
fn a_b_a_driver_preserves_each_instances_member_heap() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let a_id = engine.add_entity(scripted_soldier("HeapA"));
    let b_id = engine.add_entity(scripted_soldier("HeapB"));
    let a_handle = bind_script_actor(&mut engine, a_id, "HeapA");
    let b_handle = bind_script_actor(&mut engine, b_id, "HeapB");

    engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(a_handle),
            "TriggerRelay",
            &[b_handle],
            crate::natives::ScriptCallFrame::actor(a_handle),
        )
        .expect("A→B→A member-heap callback");

    let script = engine.scripts.mission.as_ref().unwrap();
    assert_eq!(
        script.state.globals.get(&906),
        Some(&20),
        "A heap={}, B heap={}, globals={:?}",
        script_instance_heap_word(script, a_handle),
        script_instance_heap_word(script, b_handle),
        script.state.globals
    );
    assert_eq!(script_instance_heap_word(script, a_handle), 30);
    assert_eq!(script_instance_heap_word(script, b_handle), 12);
}

#[test]
fn shared_driver_reports_missing_vm_and_depth_overflow_as_errors() {
    let (mut engine, _receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    assert_eq!(
        engine
            .call_script_vm(
                &crate::sim_rng::test_context(),
                &assets,
                super::ScriptVmKey::Actor(handle),
                "OptionalMissingMethod",
                &[],
                crate::natives::ScriptCallFrame::actor(handle),
            )
            .expect("a missing optional method is the base no-op"),
        0
    );
    let missing = ScriptHandleCodec::actor_handle_from_index(9999);
    assert!(
        engine
            .call_script_vm(
                &crate::sim_rng::test_context(),
                &assets,
                super::ScriptVmKey::Actor(missing),
                "ProcessMessage",
                &[],
                crate::natives::ScriptCallFrame::actor(missing),
            )
            .expect_err("missing required VM must not fabricate a return")
            .contains("required VM is not bound")
    );

    let recursive_id = engine.add_entity(scripted_soldier("RecursiveReceiver"));
    let recursive_handle = bind_script_actor(&mut engine, recursive_id, "RecursiveReceiver");
    let error = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(recursive_handle),
            "ProcessMessage",
            &[1, 0, 0],
            crate::natives::ScriptCallFrame::actor(recursive_handle),
        )
        .expect_err("recursive ProcessMessage must stop at the driver boundary");
    assert!(error.contains("depth limit"), "unexpected error: {error}");
    let direct_states: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| &sequence.elements)
        .filter(|element| element.command == Command::SendMessage)
        .map(|element| element.state)
        .collect();
    assert_eq!(
        direct_states
            .iter()
            .filter(|state| **state == SequenceState::Terminated)
            .count(),
        3,
        "four real VM activations are accepted; the first is direct"
    );
    assert_eq!(
        direct_states
            .iter()
            .filter(|state| **state == SequenceState::Impossible)
            .count(),
        1,
        "the fifth real VM activation is rejected"
    );

    let (mut external, _receiver, _handle) = engine_with_receiver();
    let recursive_id = external.add_entity(scripted_soldier("RecursiveReceiver"));
    let recursive_handle = bind_script_actor(&mut external, recursive_id, "RecursiveReceiver");
    let error = external
        .call_external_native(
            &crate::sim_rng::test_context(),
            &assets,
            "SendMessage",
            &[recursive_handle, 1],
        )
        .expect_err("external receiver context must not consume a VM depth slot");
    assert!(error.contains("depth limit"), "unexpected error: {error}");
    let external_states: Vec<_> = external
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| &sequence.elements)
        .filter(|element| element.command == Command::SendMessage)
        .map(|element| element.state)
        .collect();
    assert_eq!(
        external_states
            .iter()
            .filter(|state| **state == SequenceState::Terminated)
            .count(),
        4,
        "the synthetic external receiver is not a real VM activation"
    );
    assert_eq!(
        external_states
            .iter()
            .filter(|state| **state == SequenceState::Impossible)
            .count(),
        1
    );
}
