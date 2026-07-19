use super::*;

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
