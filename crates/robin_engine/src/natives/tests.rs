//! Unit tests for the script native dispatch.

use super::*;
use crate::interp::*;
use crate::vm::Instruction::*;

const TMP0: u16 = 0xC000;
const TMP4: u16 = 0xC004;
const TMP8: u16 = 0xC008;
const TMP12: u16 = 0xC00C;

/// Helper: build a program that pushes constants, calls a native, and returns the result.
fn call_native_return(index: u32, args: &[i32]) -> Vec<crate::vm::Instruction> {
    let temps = [TMP0, TMP4, TMP8, TMP12];
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

/// Run a native and return the queued deferred commands for inspection.
fn run_native_deferred(index: u32, args: &[i32]) -> (StopReason, Vec<DeferredCommand>) {
    let prog = call_native_return(index, args);
    let mut vm = Vm::new().with_host(Box::new(GameHost::new()));
    let stop = vm.run(&prog);
    let mut host_box = vm.take_host().unwrap();
    let host = host_box
        .as_any_mut()
        .downcast_mut::<GameHost>()
        .expect("host is GameHost");
    (stop, std::mem::take(&mut host.deferred_commands))
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
    host.npc_values.insert((123, 7), 456);
    host.npc_values.insert((-5, 99), -12);

    serde_json::to_value(&host).expect("save/rollback JSON value");
    let json = serde_json::to_string(&host).expect("serialize GameHost");
    let decoded: GameHost = serde_json::from_str(&json).expect("deserialize GameHost");

    assert_eq!(decoded.npc_values.get(&(123, 7)), Some(&456));
    assert_eq!(decoded.npc_values.get(&(-5, 99)), Some(&-12));
}

#[test]
fn npc_custom_values_participate_in_state_hash() {
    let mut baseline = GameHost::new();
    let mut same = GameHost::new();
    let mut changed = GameHost::new();
    baseline.npc_values.insert((123, 7), 456);
    same.npc_values.insert((123, 7), 456);
    changed.npc_values.insert((123, 7), 457);

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
    let mut host = GameHost::new();
    host.script_location_count = 2;
    host.script_point_count = 2;
    host.location_positions = vec![(0.0, 0.0), (30.0, 40.0)];
    let prog = call_native_return(
        160,
        &[
            GameHost::location_handle_from_index(0),
            GameHost::location_handle_from_index(1),
        ],
    );
    let mut vm = Vm::new().with_host(Box::new(host));
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
    host.script_point_count = 1;
    host.script_location_count = 2;
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
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(3));

    // Re-add occupants since vm takes ownership
    let mut host2 = GameHost::new();
    host2.script_point_count = 1;
    host2.script_location_count = 2;
    host2.zone_occupants.insert(
        loc,
        vec![
            GameHost::actor_handle_from_index(2),
            GameHost::actor_handle_from_index(4),
            GameHost::actor_handle_from_index(6),
        ],
    );
    let prog2 = call_native_return(205, &[loc, 1]);
    let mut vm2 = Vm::new().with_host(Box::new(host2));
    assert_eq!(
        vm2.run(&prog2),
        StopReason::ReturnedValue(GameHost::actor_handle_from_index(4))
    );
}

#[test]
fn compute_location_between() {
    let mut host = GameHost::new();
    host.script_location_count = 2;
    host.script_point_count = 2;
    host.location_positions = vec![(0.0, 0.0), (100.0, 200.0)];
    host.location_layers = vec![0, 0];
    host.location_sectors = vec![0, 0];
    let lambda_bits = 0.5f32.to_bits() as i32;
    let prog = call_native_return(
        213,
        &[
            GameHost::location_handle_from_index(0),
            GameHost::location_handle_from_index(1),
            lambda_bits,
        ],
    );
    let mut vm = Vm::new().with_host(Box::new(host));
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
    host.pc_handles = vec![1, 2, 3];
    host.zone_occupants.insert(5, vec![1, 2, 3]);
    let prog = call_native_return(230, &[5]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(1));
}

#[test]
fn are_all_pcs_inside_not_all() {
    let mut host = GameHost::new();
    host.pc_handles = vec![1, 2, 3];
    host.zone_occupants.insert(5, vec![1, 3]); // PC 2 missing
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
    let host = GameHost::new();
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&program), StopReason::ReturnedValue(42));
}

#[test]
fn campaign_value_default_zero() {
    assert_eq!(run_native(195, &[99]), StopReason::ReturnedValue(0));
}

// --- Custom NPC values ---

#[test]
fn npc_values_set_get_nonexistent_actor() {
    // SetCustomNPCValue(actor=3, id=5, value=77); return GetCustomNPCValue(actor=3, id=5).
    // Actor 3 doesn't exist in the bare GameHost, so Set warns + skips
    // the store and Get returns -1.  Round-trip with a real NPC entity
    // would need entity-setup plumbing — covered in engine integration
    // tests, not natives unit tests.
    let program = vec![
        BeginFunction {
            volatile_count: 0,
            temp_count: 3,
        },
        Aff0IConstant {
            dst: TMP0,
            constant: 3,
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
    let host = GameHost::new();
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&program), StopReason::ReturnedValue(-1));
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

    let mut host = GameHost::new();
    host.map_bbox = MapBBox::from_coords(0.0, 0.0, 1000.0, 800.0);
    let inside = (400.0, 300.0);

    // Direction 0 = facing north (-y). Actor enters from the south
    // edge walking north, so border is on y=800 and outside is below.
    let (border, outside) = host.compute_border_point(inside, 0);
    assert!((border.0 - 400.0).abs() < 0.1);
    assert!((border.1 - 800.0).abs() < 0.1);
    assert!(outside.1 > 800.0);

    // Direction 8 = facing south (+y). Border on y=0 (top edge),
    // outside above the map.
    let (border, outside) = host.compute_border_point(inside, 8);
    assert!((border.0 - 400.0).abs() < 0.1);
    assert!((border.1 - 0.0).abs() < 0.1);
    assert!(outside.1 < 0.0);

    // Direction 4 = facing east (+x). Border on x=0 (left edge),
    // outside to the left.
    let (border, outside) = host.compute_border_point(inside, 4);
    assert!((border.0 - 0.0).abs() < 0.1);
    assert!((border.1 - 300.0).abs() < 0.1);
    assert!(outside.0 < 0.0);

    // Direction 12 = facing west (-x). Border on x=1000, outside to
    // the right.
    let (border, outside) = host.compute_border_point(inside, 12);
    assert!((border.0 - 1000.0).abs() < 0.1);
    assert!((border.1 - 300.0).abs() < 0.1);
    assert!(outside.0 > 1000.0);
}

// ── GameHost campaign-value side effects ──────────────────────────

#[test]
fn game_host_add_campaign_value_ransom_credits_stat_and_queues_jingle() {
    let mut host = GameHost::new();
    host.campaign = Some(crate::campaign::Campaign::default());
    host.frame_counter = 100;

    host.add_campaign_value(crate::campaign::CampaignValue::Ransom, 250);

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
    host.frame_counter = 50;
    host.campaign.as_mut().unwrap().values[crate::campaign::CampaignValue::Ransom] = 200;

    // Lowering: no jingle.
    host.set_campaign_value(crate::campaign::CampaignValue::Ransom, 100);
    assert!(host.commands.is_empty());

    // Raising: jingle queued.
    host.set_campaign_value(crate::campaign::CampaignValue::Ransom, 500);
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
    host.frame_counter = 100;

    host.add_campaign_value(crate::campaign::CampaignValue::Score, 750);

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

fn native_sees(host: &mut GameHost, npc_index: usize, target_index: usize) -> i32 {
    let mut stack = NativeStack::default();
    stack.push_i32(GameHost::actor_handle_from_index(npc_index));
    stack.push_i32(GameHost::actor_handle_from_index(target_index));
    <GameHost as HostFunctions>::call(host, NativeFn::Sees as u32, &mut stack)
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

    assert_eq!(native_sees(&mut host, 0, 1), 0);

    host.is_forest_level = true;
    assert_eq!(native_sees(&mut host, 0, 1), 1);

    let Entity::Soldier(soldier) = host.entities[0].as_mut().unwrap() else {
        unreachable!("observer must remain a soldier")
    };
    soldier.soldier.cached_camp = Camp::Lacklandists;
    assert_eq!(native_sees(&mut host, 0, 1), 0);
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

    assert_eq!(host.ambiance, crate::engine::Ambiance::Day);
    assert_eq!(native_sees(&mut host, 0, 1), 1);

    host.ambiance = crate::engine::Ambiance::Night;
    assert_eq!(native_sees(&mut host, 0, 1), 0);
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
    host.pc_profile_map.insert(actor, profile_idx);
    host.campaign = Some(campaign);
    (host, actor)
}

fn call_set_experiences(host: &mut GameHost, actor: i32, sword: i32, bow: i32) {
    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(sword);
    stack.push_i32(bow);
    assert_eq!(
        <GameHost as HostFunctions>::call(host, NativeFn::SetExperiences as u32, &mut stack),
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
    let ret = <GameHost as HostFunctions>::call(
        &mut host,
        NativeFn::SetActionAvailable as u32,
        &mut stack,
    );
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
    let ret = <GameHost as HostFunctions>::call(
        &mut host,
        NativeFn::IsActionAvailable as u32,
        &mut stack,
    );
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
        <GameHost as HostFunctions>::call(
            &mut host,
            NativeFn::IsActionAvailable as u32,
            &mut stack
        ),
        1
    );

    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(1);
    assert_eq!(
        <GameHost as HostFunctions>::call(
            &mut host,
            NativeFn::IsActionAvailable as u32,
            &mut stack
        ),
        0
    );

    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(2);
    assert_eq!(
        <GameHost as HostFunctions>::call(
            &mut host,
            NativeFn::IsActionAvailable as u32,
            &mut stack
        ),
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
    let ret =
        <GameHost as HostFunctions>::call(&mut host, NativeFn::AddAsSubordinate as u32, &mut stack);
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
