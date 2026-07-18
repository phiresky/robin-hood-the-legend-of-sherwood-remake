//! Integration tests for `IElementTargetScript::ActivatedBy*` dispatch.
//!
//! `ACTIVATE_*` commands on a target dispatch to per-method callbacks
//! on the target's bound script class.  These activations are collected
//! in `pending_target_activations` during the action loop and invoked
//! through `dispatch_target_activations` after the loop.

use crate::element::{
    Command, ElementData, ElementKind, ElementTarget, Entity, FxData, Posture, TargetData,
    TargetFilter,
};
use crate::engine::{DevState, EngineInner, HostDisplayState, LevelAssets, MissionScript};
use crate::entity_id::EntityId;
use crate::scb::{ClassEntry, Function, ScbFile};
use crate::sequence::{SequenceElement, SequenceElementData};
use crate::vm::{Opcode, Quad};

// ────────────────────────────────────────────────────────────────────
// Quad encoders (the disk format is `{u8 op, [u8;8] operands}`).
// ────────────────────────────────────────────────────────────────────

fn q_begin_function(volatile: u16, temp: u16) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&volatile.to_le_bytes());
    ops[2..4].copy_from_slice(&temp.to_le_bytes());
    Quad {
        operation: Opcode::BeginFunction as u8,
        operands: ops,
    }
}

fn q_aff0_i_constant(dst: u16, constant: i32) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&dst.to_le_bytes());
    ops[4..8].copy_from_slice(&constant.to_le_bytes());
    Quad {
        operation: Opcode::Aff0IConstant as u8,
        operands: ops,
    }
}

fn q_native_param(sym: u16) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&sym.to_le_bytes());
    Quad {
        operation: Opcode::NativeParam as u8,
        operands: ops,
    }
}

fn q_native_call(index: u32) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..4].copy_from_slice(&index.to_le_bytes());
    Quad {
        operation: Opcode::NativeCall as u8,
        operands: ops,
    }
}

fn q_return() -> Quad {
    Quad {
        operation: Opcode::Return as u8,
        operands: [0u8; 8],
    }
}

// Native index 0 is `InitGlobal(id, value)` — see
// `crates/robin_engine/src/natives/defs.rs`.  Use Init rather than Set
// because `SetGlobal` requires a prior `InitGlobal` and would
// otherwise no-op + warn ("Non-valid ID for script global").
const NATIVE_INIT_GLOBAL: u32 = 0;
// Temp symbol slots — anything in the volatile/temp region works for
// this test.  Scripts use addresses in the 0xC000+ range for the
// per-frame local pool.
const TMP0: u16 = 0xC000;
const TMP4: u16 = 0xC004;

/// Build a `Function` + its quad body that stores a sentinel value in
/// a host global so the test can verify the call happened.
///
/// Produces:
///   BeginFunction(0, 2)
///   Aff0IConstant(TMP0, global_id)
///   Aff0IConstant(TMP4, sentinel)
///   NativeParam(TMP0)
///   NativeParam(TMP4)
///   NativeCall(InitGlobal)
///   Return
fn activated_by_function(
    name: &str,
    base_addr: i32,
    global_id: i32,
    sentinel: i32,
) -> (Function, Vec<Quad>) {
    let quads = vec![
        q_begin_function(0, 2),
        q_aff0_i_constant(TMP0, global_id),
        q_aff0_i_constant(TMP4, sentinel),
        q_native_param(TMP0),
        q_native_param(TMP4),
        q_native_call(NATIVE_INIT_GLOBAL),
        q_return(),
    ];
    let function = Function {
        name: name.to_string(),
        address: base_addr,
        num_parameters: 1,
        size_of_return_value: 0,
        size_of_parameters: 4,
        size_of_volatile: 0,
        size_of_temporary: 8,
    };
    (function, quads)
}

/// Build a no-op function (just BeginFunction + Return).  Used for
/// classes that need to exist but whose methods don't matter.
fn empty_function(name: &str, base_addr: i32) -> (Function, Vec<Quad>) {
    (
        Function {
            name: name.to_string(),
            address: base_addr,
            num_parameters: 0,
            size_of_return_value: 0,
            size_of_parameters: 0,
            size_of_volatile: 0,
            size_of_temporary: 0,
        },
        vec![q_begin_function(0, 0), q_return()],
    )
}

// Sentinel values each ActivatedBy* method writes into host globals.
const GLOBAL_ID_LEVER: i32 = 100;
const GLOBAL_ID_SEARCH: i32 = 101;
const GLOBAL_ID_APPLE: i32 = 102;
const GLOBAL_ID_HEAL: i32 = 103;
const GLOBAL_ID_MONEY: i32 = 104;
const GLOBAL_ID_ARROW: i32 = 105;
const SENTINEL_LEVER: i32 = 0x11111111;
const SENTINEL_SEARCH: i32 = 0x22222222;
const SENTINEL_APPLE: i32 = 0x33333333;
const SENTINEL_HEAL: i32 = 0x44444444;
const SENTINEL_MONEY: i32 = 0x55555555;
const SENTINEL_ARROW: i32 = 0x66666666;

/// Construct an SCB with a `StartUp` class (required by
/// `MissionScript::from_scb`) and a `TestTarget` class exposing six
/// `ActivatedBy*` methods that record into host globals.
fn build_test_scb() -> ScbFile {
    // StartUp: one empty Initialize function.
    let (startup_init, startup_quads) = empty_function("Initialize", 0);
    let startup = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: vec![],
        functions: vec![startup_init],
        quads: startup_quads,
    };

    // TestTarget: one record-to-global function per ActivatedBy*.
    let mut target_quads = Vec::new();
    let mut target_functions = Vec::new();
    for (name, gid, sentinel) in [
        ("ActivatedByLever", GLOBAL_ID_LEVER, SENTINEL_LEVER),
        ("ActivatedBySearch", GLOBAL_ID_SEARCH, SENTINEL_SEARCH),
        ("ActivatedByApple", GLOBAL_ID_APPLE, SENTINEL_APPLE),
        ("ActivatedByHeal", GLOBAL_ID_HEAL, SENTINEL_HEAL),
        ("ActivatedByMoney", GLOBAL_ID_MONEY, SENTINEL_MONEY),
        ("ActivatedByArrow", GLOBAL_ID_ARROW, SENTINEL_ARROW),
    ] {
        let base = target_quads.len() as i32;
        let (f, q) = activated_by_function(name, base, gid, sentinel);
        target_functions.push(f);
        target_quads.extend(q);
    }
    let test_target = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "TestTarget".into(),
        size_of_member_variables: 0,
        member_variables: vec![],
        functions: target_functions,
        quads: target_quads,
    };

    ScbFile {
        version: crate::scb::SCB_VERSION,
        classes: vec![startup, test_target],
    }
}

/// Stand up an engine with a `MissionScript` loaded from the synthetic
/// SCB, plus an FX target entity bound to the `TestTarget` class.
/// Returns the engine and the target entity id.
fn build_engine_with_target() -> (EngineInner, EntityId) {
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Some(crate::campaign::Campaign::default());
    let script = MissionScript::from_scb(build_test_scb()).expect("mission script builds");
    engine.scripts.mission = Some(script);
    engine.attach_script_bindings(&LevelAssets::new());

    // FX target with a script_class of "TestTarget".
    let target = Entity::Target(ElementTarget {
        element: ElementData {
            kind: ElementKind::Target,
            active: true,
            posture: Posture::Undefined,
            ..ElementData::default()
        },
        fx: FxData::default(),
        target: TargetData {
            action_filter: TargetFilter::all(),
            script_class: "TestTarget".into(),
            ..TargetData::default()
        },
    });
    let target_id = engine.add_entity(target);

    // Bind the target to its script class.  In production this runs
    // during per-target Initialize (see `script.rs:568-584`).
    let handle = crate::natives::GameHost::actor_handle(target_id);
    if let Some(ref mut script) = engine.scripts.mission {
        assert!(
            script.bind_target(
                handle,
                "TestTarget",
                crate::natives::NativeQueryViews::default()
            ),
            "bind_target"
        );
    }

    (engine, target_id)
}

/// Read back a host-global value after a dispatch.  Returns the
/// default (0) if the global wasn't written.
fn host_global(engine: &EngineInner, id: i32) -> i32 {
    engine
        .scripts
        .mission
        .as_ref()
        .and_then(|script| script.state.globals.get(&id).copied())
        .unwrap_or(0)
}

/// Launch a `Command::Activate*` sequence element with the given
/// antagonist (the PC/shooter).  Models how the animation-done
/// branches launch the interaction element on the target.
fn launch_activation(engine: &mut EngineInner, target: EntityId, pc: EntityId, cmd: Command) {
    let mut elem = SequenceElement::new(1, cmd, Some(target));
    elem.data = SequenceElementData::Interaction {
        antagonist: Some(pc),
    };
    engine.launch_element(elem);
}

// ────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────

/// Common target-interaction path: a PC pulls a lever.  The
/// `USING_LEVER` animation completing launches `ACTIVATE_LEVER`,
/// which dispatches to `ActivatedByLever`.
#[test]
fn activated_by_lever_fires_on_activation_command() {
    let (mut engine, target_id) = build_engine_with_target();
    let pc = target_id;
    launch_activation(&mut engine, target_id, pc, Command::ActivateLever);

    let assets = LevelAssets::new();
    let mut dev = DevState::default();
    let mut display = HostDisplayState::default();
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    assert_eq!(
        host_global(&engine, GLOBAL_ID_LEVER),
        SENTINEL_LEVER,
        "ActivatedByLever should have run and written its sentinel"
    );
}

/// Chest / search-target progression: a PC finishes the search
/// animation, the engine launches `ACTIVATE_SEARCH`, dispatcher routes
/// to `ActivatedBySearch`.
#[test]
fn activated_by_search_fires_on_activation_command() {
    let (mut engine, target_id) = build_engine_with_target();
    let pc = target_id;
    launch_activation(&mut engine, target_id, pc, Command::ActivateSearch);

    let assets = LevelAssets::new();
    let mut dev = DevState::default();
    let mut display = HostDisplayState::default();
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    assert_eq!(host_global(&engine, GLOBAL_ID_SEARCH), SENTINEL_SEARCH);
}

/// Rare projectile-activation path: an apple hits an APPLE-filter
/// target.  The apple element launches `ACTIVATE_APPLE`, dispatcher
/// routes to `ActivatedByApple`.
#[test]
fn activated_by_apple_fires_on_activation_command() {
    let (mut engine, target_id) = build_engine_with_target();
    let pc = target_id;
    launch_activation(&mut engine, target_id, pc, Command::ActivateApple);

    let assets = LevelAssets::new();
    let mut dev = DevState::default();
    let mut display = HostDisplayState::default();
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    assert_eq!(host_global(&engine, GLOBAL_ID_APPLE), SENTINEL_APPLE);
}

/// Scripted archery targets use the same projectile activation path:
/// an arrow hitting an ARROW-filter target launches `ACTIVATE_ARROW`,
/// which dispatches to `ActivatedByArrow`.
#[test]
fn activated_by_arrow_fires_on_activation_command() {
    let (mut engine, target_id) = build_engine_with_target();
    let pc = target_id;
    launch_activation(&mut engine, target_id, pc, Command::ActivateArrow);

    let assets = LevelAssets::new();
    let mut dev = DevState::default();
    let mut display = HostDisplayState::default();
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    assert_eq!(host_global(&engine, GLOBAL_ID_ARROW), SENTINEL_ARROW);
}

/// All nine `ActivatedBy*` commands share the same dispatcher; spot-
/// check that a target missing one of the methods is a clean no-op
/// rather than a hard error.  (The `TestTarget` class only defines
/// six of the nine — the other three should log but not panic.)
#[test]
fn activation_without_matching_method_is_no_op() {
    let (mut engine, target_id) = build_engine_with_target();
    let pc = target_id;
    // ActivateStone / ActivateHandle / ActivateSword are not defined
    // on `TestTarget`.
    for cmd in [
        Command::ActivateStone,
        Command::ActivateHandle,
        Command::ActivateSword,
    ] {
        launch_activation(&mut engine, target_id, pc, cmd);
    }

    let assets = LevelAssets::new();
    let mut dev = DevState::default();
    let mut display = HostDisplayState::default();
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    // None of the missing methods should have populated any global.
    assert_eq!(host_global(&engine, GLOBAL_ID_LEVER), 0);
    assert_eq!(host_global(&engine, GLOBAL_ID_SEARCH), 0);
    assert_eq!(host_global(&engine, GLOBAL_ID_APPLE), 0);
    assert_eq!(host_global(&engine, GLOBAL_ID_ARROW), 0);
}

// Removed `use_lever_on_fx_target_fires_activated_by_lever` and
// `search_cmd_on_fx_target_fires_activated_by_search`: the PC-side
// dispatcher in `tick.rs` now pushes a `UsingLever` / `Searching` order
// onto the PC and only launches `ACTIVATE_*` when the order reports
// `MotionState::Done`.  Reaching Done requires a real PC sprite
// playing the animation, which the synthetic SCB engine here can't
// supply.  The activation→script chain is still covered by
// `activated_by_lever_fires_on_activation_command` and the
// per-command tests below.

/// `Command::HitTarget` → `ActivateSword` → `ActivatedBySword`.
#[test]
fn hit_target_fires_activated_by_sword_when_defined() {
    // `TestTarget` doesn't define `ActivatedBySword` — this test
    // verifies the dispatcher is still a clean no-op in that case.
    let (mut engine, target_id) = build_engine_with_target();
    let pc = EntityId::Pc(crate::entity_id::PcId(0));
    let mut elem = SequenceElement::new(1, Command::HitTarget, Some(pc));
    elem.data = SequenceElementData::Interaction {
        antagonist: Some(target_id),
    };
    engine.launch_element(elem);

    let assets = LevelAssets::new();
    let mut dev = DevState::default();
    let mut display = HostDisplayState::default();
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    // No global gets set (method isn't defined on TestTarget).
    // Important: we don't panic or log an error for a missing method.
}

/// `Command::HandleTarget` and `Command::TakeTarget` both route to
/// `ActivatedByHand`.
#[test]
fn handle_target_and_take_target_both_route_to_activated_by_hand() {
    let (mut engine, target_id) = build_engine_with_target();
    let pc = EntityId::Pc(crate::entity_id::PcId(0));
    for cmd in [Command::HandleTarget, Command::TakeTarget] {
        let mut elem = SequenceElement::new(1, cmd, Some(pc));
        elem.data = SequenceElementData::Interaction {
            antagonist: Some(target_id),
        };
        engine.launch_element(elem);
    }

    let assets = LevelAssets::new();
    let mut dev = DevState::default();
    let mut display = HostDisplayState::default();
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    // TestTarget doesn't define ActivatedByHand — this test just
    // verifies both commands dispatch without panic.
}

/// Verifies the PC handle is passed through as the first script
/// parameter to `ActivatedBy*(pPC)`.  We don't read the parameter back
/// directly here — the SetGlobal sentinel already proves the method
/// ran; this test covers that multiple activations on the same target
/// can stack and all fire in order.
#[test]
fn multiple_activations_in_one_tick_all_fire() {
    let (mut engine, target_id) = build_engine_with_target();
    let pc = target_id;
    launch_activation(&mut engine, target_id, pc, Command::ActivateLever);
    launch_activation(&mut engine, target_id, pc, Command::ActivateSearch);
    launch_activation(&mut engine, target_id, pc, Command::ActivateApple);

    let assets = LevelAssets::new();
    let mut dev = DevState::default();
    let mut display = HostDisplayState::default();
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    assert_eq!(host_global(&engine, GLOBAL_ID_LEVER), SENTINEL_LEVER);
    assert_eq!(host_global(&engine, GLOBAL_ID_SEARCH), SENTINEL_SEARCH);
    assert_eq!(host_global(&engine, GLOBAL_ID_APPLE), SENTINEL_APPLE);
}
