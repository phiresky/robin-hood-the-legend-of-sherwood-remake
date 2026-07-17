//! Unit tests for the script native dispatch.

use super::*;
use crate::interp::*;
use crate::vm::Instruction::*;

const TMP0: u16 = 0xC000;
const TMP4: u16 = 0xC004;
const TMP8: u16 = 0xC008;
const TMP12: u16 = 0xC00C;
const TMP16: u16 = 0xC010;

/// Helper: build a program that pushes constants, calls a native, and returns the result.
fn call_native_return(index: u32, args: &[i32]) -> Vec<crate::vm::Instruction> {
    let temps = [TMP0, TMP4, TMP8, TMP12, TMP16];
    let temp_count = (args.len() + 1) as u16; // +1 for the return slot
    let ret_slot = temps[args.len()]; // first unused temp

    let mut prog = vec![BeginFunction {
        volatile_count: 0,
        temp_count,
    }];
    for (i, &val) in args.iter().enumerate() {
        prog.push(Aff0IConstant {
            dst: temps[i],
            constant: val,
        });
    }
    for &temp in &temps[..args.len()] {
        prog.push(NativeParam { sym: temp });
    }
    prog.push(NativeCall { index });
    prog.push(Aff1NativeGetReturn { sym: ret_slot });
    prog.push(ReturnVal { sym: ret_slot });
    prog
}

fn run_native(index: u32, args: &[i32]) -> StopReason {
    let prog = call_native_return(index, args);
    let host = GameHost::new();
    let mut vm = Vm::new().with_host(Box::new(host));
    vm.run(&prog)
}

fn call_host_native(host: &mut GameHost, native: NativeFn, stack: &mut NativeStack) -> i32 {
    <GameHost as HostFunctions>::call(host, native as u32, stack)
        .expect_return("non-nested native test")
}

fn call_host_native_with_queries(
    host: &mut GameHost,
    native: NativeFn,
    stack: &mut NativeStack,
    queries: NativeQueryViews<'_>,
) -> i32 {
    let mut state = ScriptState::default();
    let mut context = NativeContext::with_bindings(
        host,
        &mut state,
        AttachedScriptBindings::empty_ref(),
        queries,
    );
    <NativeContext<'_> as HostFunctions>::call(&mut context, native as u32, stack)
        .expect_return("non-nested native query test")
}

fn call_bound_host_native(
    host: &mut GameHost,
    bindings: &AttachedScriptBindings,
    native: NativeFn,
    stack: &mut NativeStack,
) -> i32 {
    let mut state = ScriptState::default();
    let mut context =
        NativeContext::with_bindings(host, &mut state, bindings, NativeQueryViews::default());
    <NativeContext<'_> as HostFunctions>::call(&mut context, native as u32, stack)
        .expect_return("non-nested native test")
}

struct BoundGameHost {
    host: GameHost,
    state: ScriptState,
    bindings: AttachedScriptBindings,
}

impl HostFunctions for BoundGameHost {
    fn call(&mut self, index: u32, stack: &mut NativeStack) -> NativeCallOutcome {
        NativeContext::with_bindings(
            &mut self.host,
            &mut self.state,
            &self.bindings,
            NativeQueryViews::default(),
        )
        .call(index, stack)
    }
}

/// Run a native and return the queued deferred commands for inspection.
fn run_native_deferred(index: u32, args: &[i32]) -> (StopReason, Vec<DeferredCommand>) {
    let prog = call_native_return(index, args);
    let mut vm = Vm::new().with_host(GameHost::new());
    let stop = vm.run(&prog);
    let mut host = vm.take_host();
    (stop, std::mem::take(&mut host.deferred_commands))
}

#[test]
fn send_message_native_queues_sequence_launch_payload() {
    let (stop, commands) = run_native_deferred(NativeFn::SendMessage as u32, &[0, 1234]);
    assert_eq!(stop, StopReason::ReturnedValue(0));
    assert!(matches!(
        commands.as_slice(),
        [DeferredCommand::SendMessage {
            actor: 0,
            message: 1234,
            arg1: 0,
            arg2: 0,
        }]
    ));

    let (stop, commands) = run_native_deferred(
        NativeFn::SendMessageWithArguments as u32,
        &[0, 2345, -11, 22],
    );
    assert_eq!(stop, StopReason::ReturnedValue(0));
    assert!(matches!(
        commands.as_slice(),
        [DeferredCommand::SendMessage {
            actor: 0,
            message: 2345,
            arg1: -11,
            arg2: 22,
        }]
    ));
}

#[test]
fn globals_init_set_get() {
    let program = vec![
        BeginFunction {
            volatile_count: 0,
            temp_count: 3,
        },
        Aff0IConstant {
            dst: TMP0,
            constant: 42,
        },
        Aff0IConstant {
            dst: TMP4,
            constant: 100,
        },
        NativeParam { sym: TMP0 },
        NativeParam { sym: TMP4 },
        NativeCall { index: 0 }, // InitGlobal
        Aff0IConstant {
            dst: TMP4,
            constant: 200,
        },
        NativeParam { sym: TMP0 },
        NativeParam { sym: TMP4 },
        NativeCall { index: 1 }, // SetGlobal
        NativeParam { sym: TMP0 },
        NativeCall { index: 2 }, // GetGlobal
        Aff1NativeGetReturn { sym: TMP8 },
        ReturnVal { sym: TMP8 },
    ];
    let host = GameHost::new();
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&program), StopReason::ReturnedValue(200));
}

#[test]
fn stub_returns_zero_and_logs() {
    let program = vec![
        BeginFunction {
            volatile_count: 0,
            temp_count: 2,
        },
        Aff0IConstant {
            dst: TMP0,
            constant: 5,
        },
        NativeParam { sym: TMP0 },
        NativeCall { index: 17 }, // StartDialog (stub)
        Aff1NativeGetReturn { sym: TMP4 },
        ReturnVal { sym: TMP4 },
    ];
    let host = GameHost::new();
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&program), StopReason::ReturnedValue(0));
}

#[test]
fn name_lookup() {
    assert_eq!(native_name(0), "InitGlobal");
    assert_eq!(native_name(17), "StartDialog");
    assert_eq!(native_name(74), "ThisActor");
    assert_eq!(native_name(999), "unknown");
}

#[test]
fn npc_custom_values_round_trip_through_json() {
    let mut host = GameHost::new();
    let mut npc = native_test_soldier();
    npc.npc_data_mut().unwrap().custom_values[7] = 456;
    host.entities.push(Some(npc));

    serde_json::to_value(&host).expect("save/rollback JSON value");
    let json = serde_json::to_string(&host).expect("serialize GameHost");
    let decoded: GameHost = serde_json::from_str(&json).expect("deserialize GameHost");

    assert_eq!(
        decoded.entities[0]
            .as_ref()
            .unwrap()
            .npc_data()
            .unwrap()
            .custom_values[7],
        456
    );
}

#[test]
fn npc_custom_values_participate_in_state_hash() {
    let mut baseline = GameHost::new();
    let mut same = GameHost::new();
    let mut changed = GameHost::new();
    for (host, value) in [(&mut baseline, 456), (&mut same, 456), (&mut changed, 457)] {
        let mut npc = native_test_soldier();
        npc.npc_data_mut().unwrap().custom_values[7] = value;
        host.entities.push(Some(npc));
    }

    assert_eq!(
        robin_util::state_hash::compute(&baseline),
        robin_util::state_hash::compute(&same)
    );
    assert_ne!(
        robin_util::state_hash::compute(&baseline),
        robin_util::state_hash::compute(&changed)
    );
}

#[test]
fn door_sector_goal_resolves_click_polygon_door_index() {
    let mut host = GameHost::new();
    let mut door = Door {
        active: true,
        click_polygon: vec![(10.0, 10.0), (30.0, 10.0), (30.0, 30.0), (10.0, 30.0)],
        ..Default::default()
    };
    door.rebuild_click_bbox();
    host.doors.push(door);

    assert_eq!(
        host.door_index_for_goal_sector(99, (20.0, 20.0)),
        Some(crate::gate::DoorIndex(0))
    );
}

// --- Sequence manager ---

#[test]
fn start_returns_one() {
    assert_eq!(run_native(30, &[]), StopReason::ReturnedValue(1));
}

#[test]
fn thanx_without_recording_returns_zero() {
    // Thanx with no active recording logs an error and returns false.
    assert_eq!(run_native(31, &[]), StopReason::ReturnedValue(0));
}

#[test]
fn then_outside_recording_returns_zero() {
    // Then with sequence_level < 1 logs an error and returns 0.  It
    // must not mutate any recording state — every call returns 0, not
    // an incrementing id.
    let program = vec![
        BeginFunction {
            volatile_count: 0,
            temp_count: 3,
        },
        NativeCall { index: 32 }, // Then → 0
        Aff1NativeGetReturn { sym: TMP0 },
        NativeCall { index: 32 }, // Then → 0
        Aff1NativeGetReturn { sym: TMP4 },
        NativeCall { index: 32 }, // Then → 0
        Aff1NativeGetReturn { sym: TMP8 },
        ReturnVal { sym: TMP8 },
    ];
    let host = GameHost::new();
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&program), StopReason::ReturnedValue(0));
}

// --- Actor comparison & state queries ---

#[test]
fn script_actor_handle_maps_back_to_zero_based_entity_index() {
    assert_eq!(GameHost::actor_handle_index(0), None);
    assert_eq!(
        GameHost::actor_handle_index(GameHost::actor_handle_from_index(0)),
        Some(0)
    );
    assert_eq!(
        GameHost::actor_handle_index(GameHost::actor_handle_from_index(70)),
        Some(70)
    );
}

#[test]
fn is_actor_equal_same() {
    assert_eq!(run_native(86, &[7, 7]), StopReason::ReturnedValue(1));
}

#[test]
fn is_actor_equal_different() {
    assert_eq!(run_native(86, &[7, 8]), StopReason::ReturnedValue(0));
}

#[test]
fn is_actor_dead_unknown_handle() {
    // No entity at handle 5 → default 0 (not dead).
    assert_eq!(run_native(87, &[5]), StopReason::ReturnedValue(0));
}

#[test]
fn is_actor_ko_unknown_handle() {
    assert_eq!(run_native(88, &[5]), StopReason::ReturnedValue(0));
}

#[test]
fn is_actor_tied_unknown_handle() {
    assert_eq!(run_native(89, &[5]), StopReason::ReturnedValue(0));
}

#[test]
fn is_actor_hs_unknown_handle() {
    assert_eq!(run_native(90, &[5]), StopReason::ReturnedValue(0));
}

// --- Actor action / activation ---

#[test]
fn god_returns_null_handle() {
    // God() returns NULL, which is handle 0.
    assert_eq!(run_native(111, &[]), StopReason::ReturnedValue(0));
}

#[test]
fn stop_actor_unknown_handle_noop() {
    // Invalid handle → warn, no deferred command.
    let (stop, cmds) = run_native_deferred(103, &[5]);
    assert_eq!(stop, StopReason::ReturnedValue(0));
    assert!(cmds.is_empty());
}

#[test]
fn select_select_all_queues_command() {
    // `Select` returns true unconditionally (including the error branch).
    let (stop, cmds) = run_native_deferred(112, &[31]);
    assert_eq!(stop, StopReason::ReturnedValue(1));
    assert!(matches!(
        cmds.first(),
        Some(DeferredCommand::SelectPC {
            actor: 0,
            select: true
        })
    ));
}

#[test]
fn select_unselect_all_queues_command() {
    let (stop, cmds) = run_native_deferred(112, &[0]);
    assert_eq!(stop, StopReason::ReturnedValue(1));
    assert!(matches!(
        cmds.first(),
        Some(DeferredCommand::SelectPC {
            actor: 0,
            select: false
        })
    ));
}

#[test]
fn select_unknown_code_warns_but_no_command() {
    let (stop, cmds) = run_native_deferred(112, &[5]);
    assert_eq!(stop, StopReason::ReturnedValue(1));
    assert!(cmds.is_empty());
}

#[test]
fn deactivate_unknown_handle_noop() {
    assert_eq!(run_native(113, &[3]), StopReason::ReturnedValue(0));
}

#[test]
fn activate_unknown_handle_noop() {
    assert_eq!(run_native(114, &[3]), StopReason::ReturnedValue(0));
}

// --- AI control ---

#[test]
fn lock_ai_unknown_handle_noop() {
    assert_eq!(run_native(134, &[5, 1]), StopReason::ReturnedValue(0));
}

#[test]
fn unlock_ai_unknown_handle_noop() {
    assert_eq!(run_native(135, &[5]), StopReason::ReturnedValue(0));
}

#[test]
fn freeze_unknown_handle_noop() {
    assert_eq!(run_native(138, &[5, 1]), StopReason::ReturnedValue(0));
}

#[test]
fn freeze_all_queues_command() {
    let (stop, cmds) = run_native_deferred(139, &[1]);
    assert_eq!(stop, StopReason::ReturnedValue(0));
    assert!(matches!(
        cmds.first(),
        Some(DeferredCommand::FreezeAll { freeze: true })
    ));
}

#[test]
fn freeze_all_unfreeze_queues_command() {
    let (stop, cmds) = run_native_deferred(139, &[0]);
    assert_eq!(stop, StopReason::ReturnedValue(0));
    assert!(matches!(
        cmds.first(),
        Some(DeferredCommand::FreezeAll { freeze: false })
    ));
}

// --- Location / distance ---

#[test]
fn nowhere_returns_zero() {
    assert_eq!(run_native(159, &[]), StopReason::ReturnedValue(0));
}

#[test]
fn get_distance_with_positions() {
    let host = GameHost::new();
    let bindings = AttachedScriptBindings {
        script_location_count: 2,
        script_point_count: 2,
        location_positions: std::sync::Arc::new(vec![(0.0, 0.0), (30.0, 40.0)]),
        ..Default::default()
    };
    let prog = call_native_return(
        160,
        &[
            GameHost::location_handle_from_index(0),
            GameHost::location_handle_from_index(1),
        ],
    );
    let mut vm = Vm::new().with_host(Box::new(BoundGameHost {
        host,
        state: ScriptState::default(),
        bindings,
    }));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(50)); // sqrt(30²+40²)=50
}

#[test]
fn get_distance_invalid_handle() {
    assert_eq!(run_native(160, &[99, 100]), StopReason::ReturnedValue(0));
}

#[test]
fn is_inside_building_specific() {
    let mut host = GameHost::new();
    let actor = GameHost::actor_handle_from_index(4);
    let building = GameHost::building_handle_from_index(2);
    host.actor_building.insert(actor, building);
    let prog = call_native_return(98, &[actor, building]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(1));
}

#[test]
fn is_inside_building_wrong() {
    let mut host = GameHost::new();
    let actor = GameHost::actor_handle_from_index(4);
    host.actor_building
        .insert(actor, GameHost::building_handle_from_index(2));
    let prog = call_native_return(98, &[actor, GameHost::building_handle_from_index(6)]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(0));
}

#[test]
fn is_inside_building_null_checks_any() {
    let mut host = GameHost::new();
    let actor = GameHost::actor_handle_from_index(4);
    host.actor_building
        .insert(actor, GameHost::building_handle_from_index(2));
    // NULL building (0): checks if in ANY building
    let prog = call_native_return(98, &[actor, 0]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(1));
}

#[test]
fn is_inside_building_not_in_any() {
    let host = GameHost::new();
    let prog = call_native_return(98, &[GameHost::actor_handle_from_index(4), 0]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(0));
}

#[test]
fn is_inside_zone() {
    let mut host = GameHost::new();
    let actor = GameHost::actor_handle_from_index(4);
    let loc = GameHost::location_handle_from_index(1);
    host.zone_occupants.insert(
        loc,
        vec![
            GameHost::actor_handle_from_index(2),
            actor,
            GameHost::actor_handle_from_index(6),
        ],
    );
    let prog = call_native_return(97, &[actor, loc]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(1));
}

#[test]
fn is_inside_zone_not_present() {
    let mut host = GameHost::new();
    let actor = GameHost::actor_handle_from_index(4);
    let loc = GameHost::location_handle_from_index(1);
    host.zone_occupants.insert(
        loc,
        vec![
            GameHost::actor_handle_from_index(2),
            GameHost::actor_handle_from_index(6),
        ],
    );
    let prog = call_native_return(97, &[actor, loc]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(0));
}

#[test]
fn actors_in_sector() {
    // GetNumberOfActorsInSector / GetActorInSector reject non-sector
    // handles via `is_script_sector_handle` (sector handles live in
    // `script_point_count < loc <= script_location_count`), so seed
    // counts so loc=2 is a valid sector handle.
    let mut host = GameHost::new();
    let bindings = AttachedScriptBindings {
        script_point_count: 1,
        script_location_count: 2,
        ..Default::default()
    };
    let loc = GameHost::location_handle_from_index(1);
    host.zone_occupants.insert(
        loc,
        vec![
            GameHost::actor_handle_from_index(2),
            GameHost::actor_handle_from_index(4),
            GameHost::actor_handle_from_index(6),
        ],
    );

    let prog = call_native_return(204, &[loc]);
    let mut vm = Vm::new().with_host(Box::new(BoundGameHost {
        host,
        state: ScriptState::default(),
        bindings,
    }));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(3));

    // Re-add occupants since vm takes ownership
    let mut host2 = GameHost::new();
    let bindings2 = AttachedScriptBindings {
        script_point_count: 1,
        script_location_count: 2,
        ..Default::default()
    };
    host2.zone_occupants.insert(
        loc,
        vec![
            GameHost::actor_handle_from_index(2),
            GameHost::actor_handle_from_index(4),
            GameHost::actor_handle_from_index(6),
        ],
    );
    let prog2 = call_native_return(205, &[loc, 1]);
    let mut vm2 = Vm::new().with_host(Box::new(BoundGameHost {
        host: host2,
        state: ScriptState::default(),
        bindings: bindings2,
    }));
    assert_eq!(
        vm2.run(&prog2),
        StopReason::ReturnedValue(GameHost::actor_handle_from_index(4))
    );
}

#[test]
fn compute_location_between() {
    let host = GameHost::new();
    let bindings = AttachedScriptBindings {
        script_location_count: 2,
        script_point_count: 2,
        location_positions: std::sync::Arc::new(vec![(0.0, 0.0), (100.0, 200.0)]),
        location_layers: std::sync::Arc::new(vec![0, 0]),
        location_sectors: std::sync::Arc::new(vec![0, 0]),
        ..Default::default()
    };
    let lambda_bits = 0.5f32.to_bits() as i32;
    let prog = call_native_return(
        213,
        &[
            GameHost::location_handle_from_index(0),
            GameHost::location_handle_from_index(1),
            lambda_bits,
        ],
    );
    let mut vm = Vm::new().with_host(Box::new(BoundGameHost {
        host,
        state: ScriptState::default(),
        bindings,
    }));
    // Should return a handle >= 3 (first computed location)
    match vm.run(&prog) {
        StopReason::ReturnedValue(handle) => {
            assert_eq!(GameHost::location_index(handle), Some(2));
        }
        other => panic!("expected return, got {other:?}"),
    }
}

#[test]
fn are_all_pcs_inside() {
    let mut host = GameHost::new();
    host.entities = vec![
        Some(native_test_pc(Vec::new(), Vec::new())),
        Some(native_test_pc(Vec::new(), Vec::new())),
        Some(native_test_pc(Vec::new(), Vec::new())),
    ];
    host.zone_occupants
        .insert(5, (0..3).map(GameHost::actor_handle_from_index).collect());
    let prog = call_native_return(230, &[5]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(1));
}

#[test]
fn are_all_pcs_inside_not_all() {
    let mut host = GameHost::new();
    host.entities = vec![
        Some(native_test_pc(Vec::new(), Vec::new())),
        Some(native_test_pc(Vec::new(), Vec::new())),
        Some(native_test_pc(Vec::new(), Vec::new())),
    ];
    let handles: Vec<_> = (0..3).map(GameHost::actor_handle_from_index).collect();
    host.zone_occupants.insert(5, vec![handles[0], handles[2]]); // PC 2 missing
    let prog = call_native_return(230, &[5]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(0));
}

#[test]
fn register_production_sector() {
    let host = GameHost::new();
    // RegisterAsProductionSector(type=0, loc=3, speed=10)
    let prog = call_native_return(199, &[0, 3, 10]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(0));
}

// --- Custom campaign values ---

#[test]
fn campaign_values_set_get() {
    // SetCustomCampaignValue(7, 42); return GetCustomCampaignValue(7)
    let program = vec![
        BeginFunction {
            volatile_count: 0,
            temp_count: 3,
        },
        Aff0IConstant {
            dst: TMP0,
            constant: 7,
        },
        Aff0IConstant {
            dst: TMP4,
            constant: 42,
        },
        NativeParam { sym: TMP0 },
        NativeParam { sym: TMP4 },
        NativeCall { index: 196 }, // SetCustomCampaignValue
        NativeParam { sym: TMP0 },
        NativeCall { index: 195 }, // GetCustomCampaignValue
        Aff1NativeGetReturn { sym: TMP8 },
        ReturnVal { sym: TMP8 },
    ];
    let mut host = GameHost::new();
    host.campaign = Some(crate::campaign::Campaign::default());
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&program), StopReason::ReturnedValue(42));
}

#[test]
fn campaign_value_default_zero() {
    assert_eq!(run_native(195, &[99]), StopReason::ReturnedValue(0));
}

// --- Custom NPC values ---

#[test]
fn npc_values_set_then_get_from_canonical_entity() {
    let actor = GameHost::actor_handle_from_index(0);
    // SetCustomNPCValue(actor, id=5, value=77); return GetCustomNPCValue(actor, id=5).
    let program = vec![
        BeginFunction {
            volatile_count: 0,
            temp_count: 3,
        },
        Aff0IConstant {
            dst: TMP0,
            constant: actor,
        }, // actor
        Aff0IConstant {
            dst: TMP4,
            constant: 5,
        }, // id
        Aff0IConstant {
            dst: TMP8,
            constant: 77,
        }, // value
        NativeParam { sym: TMP0 },
        NativeParam { sym: TMP4 },
        NativeParam { sym: TMP8 },
        NativeCall { index: 198 }, // SetCustomNPCValue
        NativeParam { sym: TMP0 },
        NativeParam { sym: TMP4 },
        NativeCall { index: 197 }, // GetCustomNPCValue
        Aff1NativeGetReturn { sym: TMP8 },
        ReturnVal { sym: TMP8 },
    ];
    let mut host = GameHost::new();
    host.entities.push(Some(native_test_soldier()));
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&program), StopReason::ReturnedValue(77));
}

#[test]
fn custom_values_are_isolated_between_script_hosts() {
    fn set_campaign(host: &mut GameHost, value: i32) {
        let mut stack = NativeStack::default();
        stack.push_i32(3);
        stack.push_i32(value);
        assert_eq!(
            call_host_native(host, NativeFn::SetCustomCampaignValue, &mut stack),
            0
        );
    }

    fn get_campaign(host: &mut GameHost) -> i32 {
        let mut stack = NativeStack::default();
        stack.push_i32(3);
        call_host_native(host, NativeFn::GetCustomCampaignValue, &mut stack)
    }

    let mut first = GameHost::new();
    first.campaign = Some(crate::campaign::Campaign::default());
    let mut second = GameHost::new();
    second.campaign = Some(crate::campaign::Campaign::default());

    set_campaign(&mut first, 11);
    set_campaign(&mut second, 22);

    assert_eq!(get_campaign(&mut first), 11);
    assert_eq!(get_campaign(&mut second), 22);
}

#[test]
fn deferred_selection_is_visible_to_later_natives_in_the_same_callback() {
    let mut host = GameHost::new();
    host.entities
        .push(Some(native_test_pc(Vec::new(), Vec::new())));
    let actor = GameHost::actor_handle_from_index(0);
    let sequences = crate::sequence::SequenceManager::new();
    let selected = Vec::new();
    let sounds = crate::sound_source::SoundSourceManager::new();
    let weather = crate::engine::WeatherState::default();
    let frame = 17;
    let queries = NativeQueryViews::new(&sequences, &selected, &sounds, &weather, &frame);

    let mut select = NativeStack::default();
    select.push_i32(actor);
    select.push_i32(1);
    assert_eq!(
        call_host_native_with_queries(&mut host, NativeFn::SelectActorPC, &mut select, queries),
        0
    );

    let mut is_selected = NativeStack::default();
    is_selected.push_i32(actor);
    assert_eq!(
        call_host_native_with_queries(&mut host, NativeFn::IsPCSelected, &mut is_selected, queries,),
        1
    );
    assert!(selected.is_empty(), "the canonical selection drains later");
    assert!(matches!(
        host.deferred_commands.as_slice(),
        [DeferredCommand::SelectPC {
            actor: queued_actor,
            select: true,
        }] if *queued_actor == actor
    ));
}

#[test]
fn deferred_sound_destruction_is_visible_without_mutating_the_source_manager() {
    let mut host = GameHost::new();
    let sequences = crate::sequence::SequenceManager::new();
    let mut sounds = crate::sound_source::SoundSourceManager::new();
    sounds.sources_push_some(crate::sound_source::SoundSource::default());
    let weather = crate::engine::WeatherState::default();
    let frame = 23;
    let queries = NativeQueryViews::new(&sequences, &[], &sounds, &weather, &frame);
    let handle = GameHost::sound_source_handle_from_index(0);

    let mut destroy = NativeStack::default();
    destroy.push_i32(handle);
    assert_eq!(
        call_host_native_with_queries(
            &mut host,
            NativeFn::DestroySoundSource,
            &mut destroy,
            queries,
        ),
        1
    );

    let mut lookup = NativeStack::default();
    lookup.push_i32(0);
    assert_eq!(
        call_host_native_with_queries(
            &mut host,
            NativeFn::GetSoundSourceScript,
            &mut lookup,
            queries,
        ),
        0
    );
    assert!(sounds.get(0).is_some(), "the source manager drains later");
    assert!(matches!(
        host.sound_commands.as_slice(),
        [SoundCommand::Destroy(queued)] if *queued == handle
    ));
}

#[test]
fn current_action_and_frame_queries_read_canonical_runtime_state() {
    let pc_id = EntityId::Pc(crate::entity_id::PcId(0));
    let pc_handle = GameHost::actor_handle(pc_id);
    let mut pc_host = GameHost::new();
    pc_host
        .entities
        .push(Some(native_test_pc(Vec::new(), Vec::new())));
    let mut sequences = crate::sequence::SequenceManager::new();
    let mut element =
        crate::sequence::SequenceElement::new(1, crate::element::Command::Move, Some(pc_id));
    element.push_order(crate::order::Order::new(
        crate::order::OrderType::RunningUpright,
        0.0,
        0.0,
        std::num::NonZeroU32::new(1).unwrap(),
    ));
    let sequence_id = sequences.launch_element(element);
    sequences.element_in_progress(sequence_id, 0);
    let sounds = crate::sound_source::SoundSourceManager::new();
    let weather = crate::engine::WeatherState::default();
    let frame = 123;
    let queries = NativeQueryViews::new(&sequences, &[], &sounds, &weather, &frame);

    let mut action = NativeStack::default();
    action.push_i32(pc_handle);
    assert_eq!(
        call_host_native_with_queries(
            &mut pc_host,
            NativeFn::GetCurrentAction,
            &mut action,
            queries,
        ),
        crate::order::OrderType::RunningUpright as i32
    );

    let mut npc_host = GameHost::new();
    npc_host.entities.push(Some(native_test_soldier()));
    let mut emoticon = NativeStack::default();
    emoticon.push_i32(GameHost::actor_handle_from_index(0));
    emoticon.push_i32(crate::ai::EmoticonType::QuestionMark as i32);
    emoticon.push_i32(7);
    assert_eq!(
        call_host_native_with_queries(
            &mut npc_host,
            NativeFn::SetNPCEmoticon,
            &mut emoticon,
            queries,
        ),
        0
    );
    assert_eq!(
        npc_host.entities[0]
            .as_ref()
            .unwrap()
            .ai_controller()
            .unwrap()
            .emoticon_expiration_date,
        130
    );
}

#[test]
fn canonical_query_views_are_isolated_between_engine_instances() {
    let first_sequences = crate::sequence::SequenceManager::new();
    let first_selection = [EntityId::Pc(crate::entity_id::PcId(0))];
    let first_sounds = crate::sound_source::SoundSourceManager::new();
    let first_weather = crate::engine::WeatherState::default();
    let first_frame = 10;
    let second_sequences = crate::sequence::SequenceManager::new();
    let second_selection = [
        EntityId::Pc(crate::entity_id::PcId(0)),
        EntityId::Pc(crate::entity_id::PcId(1)),
    ];
    let second_sounds = crate::sound_source::SoundSourceManager::new();
    let second_weather = crate::engine::WeatherState::default();
    let second_frame = 900;
    let first_queries = NativeQueryViews::new(
        &first_sequences,
        &first_selection,
        &first_sounds,
        &first_weather,
        &first_frame,
    );
    let second_queries = NativeQueryViews::new(
        &second_sequences,
        &second_selection,
        &second_sounds,
        &second_weather,
        &second_frame,
    );
    let mut first_host = GameHost::new();
    let mut second_host = GameHost::new();

    assert_eq!(
        call_host_native_with_queries(
            &mut first_host,
            NativeFn::GetNumberOfSelectedPCs,
            &mut NativeStack::default(),
            first_queries,
        ),
        1
    );
    assert_eq!(
        call_host_native_with_queries(
            &mut second_host,
            NativeFn::GetNumberOfSelectedPCs,
            &mut NativeStack::default(),
            second_queries,
        ),
        2
    );
}

#[test]
fn legacy_query_mirrors_are_ignored_when_loading_game_host_json() {
    let mut value = serde_json::to_value(GameHost::new()).expect("serialize GameHost");
    let object = value
        .as_object_mut()
        .expect("GameHost serializes as an object");
    for (field, old_value) in [
        ("current_animations", serde_json::json!({"123": 7})),
        ("selected_pc_handles", serde_json::json!([123, 456])),
        ("sound_source_alive", serde_json::json!([true, false])),
        ("sound_source_count", serde_json::json!(2)),
        ("ambiance", serde_json::json!("Night")),
        ("is_forest_level", serde_json::json!(true)),
        ("frame_counter", serde_json::json!(9876)),
    ] {
        object.insert(field.into(), old_value);
    }

    let mut restored: GameHost =
        serde_json::from_value(value).expect("unknown legacy mirror fields are ignored");
    let saved_again = serde_json::to_value(&restored).expect("re-serialize GameHost");
    for field in [
        "current_animations",
        "selected_pc_handles",
        "sound_source_alive",
        "sound_source_count",
        "ambiance",
        "is_forest_level",
        "frame_counter",
    ] {
        assert!(
            saved_again.get(field).is_none(),
            "legacy field {field} returned"
        );
    }

    let sequences = crate::sequence::SequenceManager::new();
    let selection = [EntityId::Pc(crate::entity_id::PcId(4))];
    let sounds = crate::sound_source::SoundSourceManager::new();
    let weather = crate::engine::WeatherState::default();
    let frame = 4;
    assert_eq!(
        call_host_native_with_queries(
            &mut restored,
            NativeFn::GetNumberOfSelectedPCs,
            &mut NativeStack::default(),
            NativeQueryViews::new(&sequences, &selection, &sounds, &weather, &frame),
        ),
        1,
        "loaded hosts query canonical runtime state, not stale save mirrors"
    );
}

#[test]
fn animation_state_write_is_immediately_visible_from_canonical_entity() {
    let actor = GameHost::actor_handle_from_index(0);
    let mut host = GameHost::new();
    host.entities
        .push(Some(Entity::Fx(crate::element::ElementFx {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::Fx,
                ..Default::default()
            },
            fx: crate::element::FxData::default(),
        })));

    let mut set = NativeStack::default();
    set.push_i32(actor);
    set.push_i32(1);
    assert_eq!(
        call_host_native(&mut host, NativeFn::SetAnimationState, &mut set),
        1
    );

    let mut get = NativeStack::default();
    get.push_i32(actor);
    assert_eq!(
        call_host_native(&mut host, NativeFn::IsAnimationActive, &mut get),
        1
    );
    assert!(host.entities[0].as_ref().unwrap().element_data().active);
}

#[test]
fn npc_value_nonexistent_actor_returns_minus_one() {
    // `GetCustomNPCValue` emits an error and returns -1 when
    // ActorExists fails.  Without entity setup the actor handle
    // resolves to no entity, so we exercise that error path.
    assert_eq!(run_native(197, &[1, 1]), StopReason::ReturnedValue(-1));
}

/// Verify `compute_border_point`: given an inside point and a facing
/// direction, the border is on the edge opposite the direction of
/// travel, and the outside point sits comfortably past that edge
/// (actor silhouette no longer overlaps the map box).
#[test]
fn compute_border_point_cardinal_directions() {
    use crate::coordinates::MapBBox;

    let map_bbox = MapBBox::from_coords(0.0, 0.0, 1000.0, 800.0);
    let inside = (400.0, 300.0);

    // Direction 0 = facing north (-y). Actor enters from the south
    // edge walking north, so border is on y=800 and outside is below.
    let (border, outside) = compute_border_point_bbox(map_bbox, inside, 0);
    assert!((border.0 - 400.0).abs() < 0.1);
    assert!((border.1 - 800.0).abs() < 0.1);
    assert!(outside.1 > 800.0);

    // Direction 8 = facing south (+y). Border on y=0 (top edge),
    // outside above the map.
    let (border, outside) = compute_border_point_bbox(map_bbox, inside, 8);
    assert!((border.0 - 400.0).abs() < 0.1);
    assert!((border.1 - 0.0).abs() < 0.1);
    assert!(outside.1 < 0.0);

    // Direction 4 = facing east (+x). Border on x=0 (left edge),
    // outside to the left.
    let (border, outside) = compute_border_point_bbox(map_bbox, inside, 4);
    assert!((border.0 - 0.0).abs() < 0.1);
    assert!((border.1 - 300.0).abs() < 0.1);
    assert!(outside.0 < 0.0);

    // Direction 12 = facing west (-x). Border on x=1000, outside to
    // the right.
    let (border, outside) = compute_border_point_bbox(map_bbox, inside, 12);
    assert!((border.0 - 1000.0).abs() < 0.1);
    assert!((border.1 - 300.0).abs() < 0.1);
    assert!(outside.0 > 1000.0);
}

// ── GameHost campaign-value side effects ──────────────────────────

#[test]
fn game_host_add_campaign_value_ransom_credits_stat_and_queues_jingle() {
    let mut host = GameHost::new();
    host.campaign = Some(crate::campaign::Campaign::default());
    host.add_campaign_value(crate::campaign::CampaignValue::Ransom, 250, 100);

    assert_eq!(
        host.campaign
            .as_ref()
            .unwrap()
            .get_value(crate::campaign::CampaignValue::Ransom),
        crate::campaign::INITIAL_RANSOM + 250
    );
    assert_eq!(host.mission_stat.collected_money, 250);
    let jingle_count = host
        .commands
        .iter()
        .filter(|c| matches!(c, EngineCommand::PlayJingle(crate::sound::Jingle::CashWon)))
        .count();
    assert_eq!(jingle_count, 1);
}

#[test]
fn game_host_set_campaign_value_ransom_jingle_only_when_growing() {
    let mut host = GameHost::new();
    host.campaign = Some(crate::campaign::Campaign::default());
    host.campaign.as_mut().unwrap().values[crate::campaign::CampaignValue::Ransom] = 200;

    // Lowering: no jingle.
    host.set_campaign_value(crate::campaign::CampaignValue::Ransom, 100, 50);
    assert!(host.commands.is_empty());

    // Raising: jingle queued.
    host.set_campaign_value(crate::campaign::CampaignValue::Ransom, 500, 50);
    let jingle_count = host
        .commands
        .iter()
        .filter(|c| matches!(c, EngineCommand::PlayJingle(crate::sound::Jingle::CashWon)))
        .count();
    assert_eq!(jingle_count, 1);
    // SetValue does NOT credit collected_money.
    assert_eq!(host.mission_stat.collected_money, 0);
}

#[test]
fn game_host_add_campaign_value_score_credits_added_score_silently() {
    let mut host = GameHost::new();
    host.campaign = Some(crate::campaign::Campaign::default());
    host.add_campaign_value(crate::campaign::CampaignValue::Score, 750, 100);

    assert_eq!(host.mission_stat.added_score, 750);
    assert!(host.commands.is_empty());
}

fn native_test_soldier() -> Entity {
    Entity::Soldier(crate::element::ActorSoldier {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorSoldier,
            ..Default::default()
        },
        actor: crate::element::ActorData::default(),
        human: crate::element::HumanData::default(),
        npc: crate::element::NpcData {
            ai_brain: crate::element::AiBrain::Enemy(Box::new(crate::ai_enemy::EnemyAi::new(0))),
            ..Default::default()
        },
        soldier: crate::element::SoldierData::default(),
    })
}

fn native_test_pc(disabled_actions: Vec<bool>, disabled_actions_temp: Vec<bool>) -> Entity {
    Entity::Pc(crate::element::ActorPc {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorPc,
            ..Default::default()
        },
        actor: crate::element::ActorData::default(),
        human: crate::element::HumanData::default(),
        pc: crate::element::PcData {
            disabled_actions,
            disabled_actions_temp,
            ..Default::default()
        },
    })
}

fn persistent_property_test_host(with_campaign: bool) -> (GameHost, AttachedScriptBindings, i32) {
    use crate::profiles::{Action, CharacterProfile, CharacterProfileIdx};

    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.characters.push(CharacterProfile {
        actions: [Action::Bow, Action::Stone, Action::Apple],
        action_max_ammo: [12, 6, 6],
        ..Default::default()
    });
    let mut host = GameHost::new();
    let bindings = AttachedScriptBindings {
        profile_manager: std::sync::Arc::new(profiles),
        ..Default::default()
    };

    let mut pc = native_test_pc(vec![true; 3], vec![false; 3]);
    let pc_data = pc.pc_data_mut().expect("test entity must be a PC");
    pc_data.profile_index = CharacterProfileIdx(0);
    pc_data.current_action = Action::Bow;
    pc_data.saved_action = Action::Bow;
    host.entities = vec![Some(pc)];

    if with_campaign {
        let mut status = crate::pc_status::PcStatus::default();
        status.set_ammo(Action::Bow, 2);
        status.set_ammo(Action::Stone, 5);
        host.campaign = Some(crate::campaign::Campaign {
            characters: vec![crate::campaign::PcDescription {
                character_profile_idx: Some(CharacterProfileIdx(0)),
                status,
                ..Default::default()
            }],
            ..Default::default()
        });
    }

    (host, bindings, GameHost::actor_handle_from_index(0))
}

fn call_set_persistent_property(
    host: &mut GameHost,
    bindings: &AttachedScriptBindings,
    actor: i32,
    prop: i32,
    amount: i32,
) -> i32 {
    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(prop);
    stack.push_i32(amount);
    call_bound_host_native(host, bindings, NativeFn::SetPersistentProperty, &mut stack)
}

fn call_get_persistent_property(
    host: &mut GameHost,
    bindings: &AttachedScriptBindings,
    actor: i32,
    prop: i32,
) -> i32 {
    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(prop);
    call_bound_host_native(host, bindings, NativeFn::GetPersistentProperty, &mut stack)
}

#[test]
fn set_persistent_property_updates_live_pc_ammo_without_campaign() {
    use crate::element::PcAmmoData;
    use crate::profiles::Action;

    let (mut host, bindings, actor) = persistent_property_test_host(false);

    assert_eq!(
        call_set_persistent_property(&mut host, &bindings, actor, 0, 7),
        1
    );
    assert_eq!(
        call_set_persistent_property(&mut host, &bindings, actor, 5, 4),
        1
    );

    let pc = host.entities[0].as_ref().unwrap().pc_data().unwrap();
    assert_eq!(
        pc.ammo,
        PcAmmoData {
            arrows: 7,
            stones: 4,
            ..Default::default()
        }
    );
    assert_eq!(pc.disabled_actions, [false, false, true]);
    assert_eq!(pc.current_action, Action::Bow);
    assert_eq!(pc.saved_action, Action::Bow);
    assert_eq!(
        call_get_persistent_property(&mut host, &bindings, actor, 0),
        7
    );
    assert_eq!(
        call_get_persistent_property(&mut host, &bindings, actor, 5),
        4
    );
}

#[test]
fn set_persistent_property_updates_live_and_campaign_pc_ammo() {
    use crate::element::PcAmmoData;
    use crate::profiles::Action;

    let (mut host, bindings, actor) = persistent_property_test_host(true);
    {
        let pc = host.entities[0].as_mut().unwrap().pc_data_mut().unwrap();
        pc.ammo.arrows = 2;
        pc.ammo.stones = 5;
        pc.current_action = Action::Stone;
        pc.saved_action = Action::Stone;
    }

    assert_eq!(
        call_set_persistent_property(&mut host, &bindings, actor, 0, 6),
        1
    );
    assert_eq!(
        call_set_persistent_property(&mut host, &bindings, actor, 5, 0),
        1
    );

    let pc = host.entities[0].as_ref().unwrap().pc_data().unwrap();
    assert_eq!(
        pc.ammo,
        PcAmmoData {
            arrows: 6,
            ..Default::default()
        }
    );
    assert_eq!(pc.disabled_actions, [false, true, true]);
    assert_eq!(pc.current_action, Action::NoAction);
    assert_eq!(pc.saved_action, Action::NoAction);

    let campaign = host.campaign.as_ref().unwrap();
    let status = &campaign.characters[0].status;
    assert_eq!(status.get_ammo(Action::Bow), 6);
    assert_eq!(status.get_ammo(Action::Stone), 0);
    assert_eq!(
        call_get_persistent_property(&mut host, &bindings, actor, 0),
        6
    );
    assert_eq!(
        call_get_persistent_property(&mut host, &bindings, actor, 5),
        0
    );
}

fn native_sees(
    host: &mut GameHost,
    weather: &crate::engine::WeatherState,
    npc_index: usize,
    target_index: usize,
) -> i32 {
    let sequences = crate::sequence::SequenceManager::new();
    let sounds = crate::sound_source::SoundSourceManager::new();
    let frame = 0;
    let mut stack = NativeStack::default();
    stack.push_i32(GameHost::actor_handle_from_index(npc_index));
    stack.push_i32(GameHost::actor_handle_from_index(target_index));
    call_host_native_with_queries(
        host,
        NativeFn::Sees,
        &mut stack,
        NativeQueryViews::new(&sequences, &[], &sounds, weather, &frame),
    )
}

fn native_sees_host(target: crate::coordinates::MapPoint, camp: Camp) -> GameHost {
    let mut npc = native_test_soldier();
    npc.element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::ZERO);
    npc.element_data_mut().set_direction_instantly(4);
    npc.element_data_mut().posture = Posture::Upright;
    let npc_data = npc.npc_data_mut().expect("test soldier has NPC data");
    npc_data.view_radius = 400;
    npc_data.eye_status = crate::element::EyeStatus::LookForward;
    npc_data.view_direction = [1.0, 0.0];
    npc_data.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    let Entity::Soldier(soldier) = &mut npc else {
        unreachable!("native_test_soldier must return a soldier")
    };
    soldier.soldier.cached_camp = camp;

    let mut pc = native_test_pc(Vec::new(), Vec::new());
    pc.element_data_mut().set_position_map(target);
    pc.element_data_mut().posture = Posture::Upright;

    let mut host = GameHost::new();
    host.entities = vec![Some(npc), Some(pc)];
    host
}

#[test]
fn sees_uses_forest_royalist_180_degree_rule() {
    // A target due south is outside an east-facing 0.5-radian cone but
    // inside the flat forward 180-degree half-plane (dot product == 0).
    let mut host = native_sees_host(
        crate::coordinates::MapPoint::new(0.0, 100.0),
        Camp::Royalists,
    );
    let mut weather = crate::engine::WeatherState::default();

    assert_eq!(native_sees(&mut host, &weather, 0, 1), 0);

    weather.is_forest_level = true;
    assert_eq!(native_sees(&mut host, &weather, 0, 1), 1);

    let Entity::Soldier(soldier) = host.entities[0].as_mut().unwrap() else {
        unreachable!("observer must remain a soldier")
    };
    soldier.soldier.cached_camp = Camp::Lacklandists;
    assert_eq!(native_sees(&mut host, &weather, 0, 1), 0);
}

#[test]
fn sees_uses_ambiance_adjusted_view_radius() {
    // With a 500-unit raw radius, a target 450 units ahead is visible in
    // day ambiance. At night the nearby light sector drives the original
    // ComputeViewRadius blend to the 400-unit day shadow-polygon radius,
    // making that same target invisible. This exercises native Sees all the
    // way through the shared compute_view_radius + compute_visibility path.
    let mut host = native_sees_host(
        crate::coordinates::MapPoint::new(450.0, 0.0),
        Camp::Lacklandists,
    );
    let mut weather = crate::engine::WeatherState::default();
    host.entities[0]
        .as_mut()
        .unwrap()
        .npc_data_mut()
        .unwrap()
        .view_radius = 500;

    let level = std::sync::Arc::make_mut(&mut host.fast_grid.level);
    level.sectors.push(crate::fast_find_grid::GridSector {
        points: vec![
            crate::coordinates::MapPoint::new(240.0, -10.0),
            crate::coordinates::MapPoint::new(260.0, -10.0),
            crate::coordinates::MapPoint::new(260.0, 10.0),
            crate::coordinates::MapPoint::new(240.0, 10.0),
        ],
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type: crate::sector::SectorType::SHADOW,
        layer: 0,
        sector_number: crate::sector::SectorNumber::new(1),
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
    level.shadow_data.insert(
        0,
        crate::sector::ShadowData {
            barycentre_2d: crate::coordinates::MapPoint::new(250.0, 0.0),
            barycentre_3d_x: 250.0,
            barycentre_3d_y: 0.0,
            barycentre_3d_z: 45.0,
            radius: 10.0,
        },
    );

    assert_eq!(weather.ambiance, crate::engine::Ambiance::Day);
    assert_eq!(native_sees(&mut host, &weather, 0, 1), 1);

    weather.ambiance = crate::engine::Ambiance::Night;
    assert_eq!(native_sees(&mut host, &weather, 0, 1), 0);
}

fn set_experiences_test_host() -> (GameHost, i32) {
    let actor = GameHost::actor_handle_from_index(0);
    let profile_idx = crate::profiles::CharacterProfileIdx(0);
    let mut status = crate::pc_status::PcStatus::default();
    status.human_status.hand_to_hand = crate::pc_status::Skill {
        experience: 37,
        capacity: 11,
    };
    status.human_status.bow = crate::pc_status::Skill {
        experience: 83,
        capacity: 22,
    };

    let mut campaign = crate::campaign::Campaign::default();
    campaign.characters.push(crate::campaign::PcDescription {
        character_profile_idx: Some(profile_idx),
        instanced: true,
        status,
    });

    let mut host = GameHost::new();
    host.entities = vec![Some(native_test_pc(Vec::new(), Vec::new()))];
    host.campaign = Some(campaign);
    (host, actor)
}

fn call_set_experiences(host: &mut GameHost, actor: i32, sword: i32, bow: i32) {
    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(sword);
    stack.push_i32(bow);
    assert_eq!(
        call_host_native(host, NativeFn::SetExperiences, &mut stack),
        0
    );
}

#[test]
fn set_experiences_updates_exact_backing_status_for_live_pc() {
    let (mut host, actor) = set_experiences_test_host();

    call_set_experiences(&mut host, actor, 64, 29);

    let status = &host.campaign.as_ref().unwrap().characters[0].status;
    assert_eq!(status.human_status.hand_to_hand.capacity, 64);
    assert_eq!(status.human_status.hand_to_hand.experience, 37);
    assert_eq!(status.human_status.bow.capacity, 29);
    assert_eq!(status.human_status.bow.experience, 83);
}

#[test]
fn set_experiences_capacities_persist_with_campaign_description() {
    let (mut host, actor) = set_experiences_test_host();
    call_set_experiences(&mut host, actor, 73, 41);

    let encoded = serde_json::to_string(host.campaign.as_ref().unwrap())
        .expect("serialize campaign after SetExperiences");
    let restored: crate::campaign::Campaign =
        serde_json::from_str(&encoded).expect("restore serialized campaign");

    let status = &restored.characters[0].status;
    assert_eq!(status.human_status.hand_to_hand.capacity, 73);
    assert_eq!(status.human_status.hand_to_hand.experience, 37);
    assert_eq!(status.human_status.bow.capacity, 41);
    assert_eq!(status.human_status.bow.experience, 83);
}

#[test]
fn set_action_available_validates_but_does_not_mutate_disabled_actions() {
    let mut host = GameHost::new();
    host.entities = vec![Some(native_test_pc(
        vec![false, false, false],
        vec![false, false, false],
    ))];

    let mut stack = NativeStack::default();
    stack.push_i32(GameHost::actor_handle_from_index(0));
    stack.push_i32(0);
    stack.push_i32(0);
    let ret = call_host_native(&mut host, NativeFn::SetActionAvailable, &mut stack);
    assert_eq!(ret, 1);
    let pc = host.entities[0].as_ref().unwrap().pc_data().unwrap();
    assert_eq!(pc.disabled_actions, [false, false, false]);
}

#[test]
fn is_action_available_rejects_out_of_range_slot() {
    let mut host = GameHost::new();
    host.entities = vec![Some(native_test_pc(
        vec![false, false, false],
        vec![false, false, false],
    ))];

    let mut stack = NativeStack::default();
    stack.push_i32(GameHost::actor_handle_from_index(0));
    stack.push_i32(-1);
    let ret = call_host_native(&mut host, NativeFn::IsActionAvailable, &mut stack);
    assert_eq!(ret, 0);
}

#[test]
fn is_action_available_reads_persistent_and_temp_slot_masks() {
    let mut host = GameHost::new();
    host.entities = vec![Some(native_test_pc(
        vec![false, true, false],
        vec![false, false, true],
    ))];
    let actor = GameHost::actor_handle_from_index(0);

    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(0);
    assert_eq!(
        call_host_native(&mut host, NativeFn::IsActionAvailable, &mut stack),
        1
    );

    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(1);
    assert_eq!(
        call_host_native(&mut host, NativeFn::IsActionAvailable, &mut stack),
        0
    );

    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(2);
    assert_eq!(
        call_host_native(&mut host, NativeFn::IsActionAvailable, &mut stack),
        0
    );
}

#[test]
fn add_as_subordinate_requests_patrol_reinit() {
    let mut host = GameHost::new();
    host.entities = vec![Some(native_test_soldier()), Some(native_test_soldier())];

    let mut stack = NativeStack::default();
    stack.push_i32(GameHost::actor_handle_from_index(0));
    stack.push_i32(GameHost::actor_handle_from_index(1));
    let ret = call_host_native(&mut host, NativeFn::AddAsSubordinate, &mut stack);
    assert_eq!(ret, 0);

    let chief_ai = host.entities[0]
        .as_ref()
        .and_then(|entity| entity.ai_controller())
        .expect("chief has AI");
    assert_eq!(
        chief_ai.theoretical_patrol,
        vec![EntityId::Soldier(crate::entity_id::SoldierId(1))]
    );
    assert!(chief_ai.patrol.is_empty());
    assert!(chief_ai.missed_patrol_members.is_empty());
    assert!(chief_ai.needs_patrol_reinit);
}
