//! Tests for on-demand `FilterAIEvent` dispatch
//! ([`Engine::filter_stimulus`] + [`Engine::dispatch_filtered_stimulus`]).
//!
//! Before each `think()` runs on a scripted NPC, the engine calls
//! `FilterAIEvent(stimulus_actor, event_code)` and skips the think if
//! the script returned zero.  The filter runs live per stimulus (no
//! precompute cache) so the script sees the actual source actor.
//!
//! Covered cases:
//!  * Mapped stimulus, source-dependent branching: filter returns the
//!    source param, so `dispatch_filtered_stimulus(sim, robin, code)` is
//!    allowed (Robin's handle is non-zero) but
//!    `filter_stimulus(sim, …, {source=0}) == false` (blocked).
//!  * Unmapped stimulus type: `filter_stimulus` calls the script with the
//!    original sentinel event code `-2`.
//!  * Missing FilterAIEvent override: the base class's implicit
//!    `return 1` must be honoured, while a missing required VM remains an
//!    error in the shared driver.
//!  * Side effects: the filter can observe-and-mutate state each call
//!    (the raison d'être for on-demand vs. precompute).

use crate::coordinates::WorldPoint3D;
use crate::element::{
    ActorCivilian, ActorData, ActorPc, ActorSoldier, AiBrain, CivilianData, ElementData,
    ElementKind, Entity, EntityId, HumanData, NpcData, PcData, Posture, SoldierData,
};
use crate::engine::EngineInner;
use crate::engine::types::{LevelAssets, MissionScript};
use crate::scb::{ClassEntry, Function, ScbFile};
use crate::vm::{Opcode, Quad};

// ───────── Quad encoders ─────────

fn q_begin_function(volatile: u16, temp: u16) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&volatile.to_le_bytes());
    ops[2..4].copy_from_slice(&temp.to_le_bytes());
    Quad {
        operation: Opcode::BeginFunction as u8,
        operands: ops,
    }
}

fn q_end_function() -> Quad {
    Quad {
        operation: Opcode::EndFunction as u8,
        operands: [0u8; 8],
    }
}

fn q_return() -> Quad {
    Quad {
        operation: Opcode::Return as u8,
        operands: [0u8; 8],
    }
}

fn q_return_val(sym: u16) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&sym.to_le_bytes());
    Quad {
        operation: Opcode::ReturnVal as u8,
        operands: ops,
    }
}

fn q_aff1_get_param(dst: u16, param_offset: i32) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&dst.to_le_bytes());
    ops[4..8].copy_from_slice(&param_offset.to_le_bytes());
    Quad {
        operation: Opcode::Aff1GetParam as u8,
        operands: ops,
    }
}

const TMP0: u16 = 0xC000;

// ───────── Synthetic classes ─────────
//
// `SourceSensitive` — `FilterAIEvent` returns `param[0]` (the source
// actor handle).  0 → blocked; non-zero → allowed.  Models the shape
// of the shipped class whose filter body branches on the source actor
// (the YellowKnight class in `S03_FoB_MP` is the only example).
//
// `NoOverride` — inherits the implicit base-class `FilterAIEvent
// { return 1; }` by simply not defining the function.

fn stub_fn(name: &str, addr: i32) -> (Function, Vec<Quad>) {
    (
        Function {
            name: name.into(),
            address: addr,
            num_parameters: 0,
            size_of_return_value: 0,
            size_of_parameters: 0,
            size_of_volatile: 0,
            size_of_temporary: 0,
        },
        vec![q_begin_function(0, 0), q_return(), q_end_function()],
    )
}

fn build_scb() -> ScbFile {
    // Source-sensitive class: real Initialize stub + FilterAIEvent
    // that returns param[0].
    let mut source_quads = Vec::new();
    let mut source_functions = Vec::new();
    for name in [
        "Initialize",
        "ActionChange",
        "HandleEvent",
        "ProcessMessage",
    ] {
        let base = source_quads.len() as i32;
        let (f, q) = stub_fn(name, base);
        source_functions.push(f);
        source_quads.extend(q);
    }
    let filter_addr = source_quads.len() as i32;
    source_functions.push(Function {
        name: "FilterAIEvent".into(),
        address: filter_addr,
        num_parameters: 3,
        size_of_return_value: 4,
        size_of_parameters: 12,
        size_of_volatile: 0,
        size_of_temporary: 4,
    });
    source_quads.push(q_begin_function(0, 1));
    source_quads.push(q_aff1_get_param(TMP0, 0)); // read source (param[0])
    source_quads.push(q_return_val(TMP0));
    source_quads.push(q_end_function());

    let source_sensitive = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "SourceSensitive".into(),
        size_of_member_variables: 0,
        member_variables: vec![],
        functions: source_functions,
        quads: source_quads,
    };

    // No-override class: stubs only, no FilterAIEvent.
    let mut noov_quads = Vec::new();
    let mut noov_functions = Vec::new();
    for name in [
        "Initialize",
        "ActionChange",
        "HandleEvent",
        "ProcessMessage",
    ] {
        let base = noov_quads.len() as i32;
        let (f, q) = stub_fn(name, base);
        noov_functions.push(f);
        noov_quads.extend(q);
    }
    let no_override = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "NoOverride".into(),
        size_of_member_variables: 0,
        member_variables: vec![],
        functions: noov_functions,
        quads: noov_quads,
    };

    let startup = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: vec![],
        functions: vec![],
        quads: vec![],
    };

    ScbFile {
        version: crate::scb::SCB_VERSION,
        classes: vec![startup, source_sensitive, no_override],
    }
}

// ───────── Engine fixture ─────────

fn make_pc(robin: bool) -> Entity {
    let mut element = ElementData {
        kind: ElementKind::ActorPc,
        active: true,
        posture: Posture::Upright,
        ..ElementData::default()
    };
    element.set_position(WorldPoint3D::default());
    Entity::Pc(ActorPc {
        element,
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: PcData {
            life_points: 50,
            robin,
            ..PcData::default()
        },
    })
}

fn make_scripted_soldier(script_class: &str) -> Entity {
    Entity::Soldier(ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        },
        actor: ActorData {
            script_class: script_class.into(),
            ..ActorData::default()
        },
        human: HumanData::default(),
        npc: NpcData {
            life_points: 50,
            ai_brain: AiBrain::Enemy(Box::default()),
            ..NpcData::default()
        },
        soldier: SoldierData::default(),
    })
}

/// Returns the engine plus the actor script handles for: robin PC, a
/// source-sensitive NPC, and a no-override NPC.
fn build_engine() -> (EngineInner, i32, i32, i32) {
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    let script = MissionScript::from_scb(build_scb()).expect("mission script builds");
    engine.scripts.mission = Some(script);
    engine.attach_script_bindings(&LevelAssets::new());

    let robin_id = engine.add_entity(make_pc(true));
    let sensitive_id = engine.add_entity(make_scripted_soldier("SourceSensitive"));
    let noov_id = engine.add_entity(make_scripted_soldier("NoOverride"));

    let robin_handle = crate::natives::ScriptHandleCodec::actor_handle(robin_id);
    let sensitive_handle = crate::natives::ScriptHandleCodec::actor_handle(sensitive_id);
    let noov_handle = crate::natives::ScriptHandleCodec::actor_handle(noov_id);

    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut engine.world.entities,
        &mut engine.ai.global,
        &mut engine.world.fast_grid,
    );
    if let Some(ref mut s) = engine.scripts.mission {
        assert!(s.bind_actor(
            sensitive_handle,
            "SourceSensitive",
            &mut engine.script_domains,
            &capabilities,
        ));
        assert!(s.bind_actor(
            noov_handle,
            "NoOverride",
            &mut engine.script_domains,
            &capabilities,
        ));
    }

    (engine, robin_handle, sensitive_handle, noov_handle)
}

// ───────── Tests ─────────

/// `filter_stimulus` returns `true` (allow) when the script call
/// returns non-zero.  Our SourceSensitive script returns
/// `param[0]` == source handle; with Robin as source, the handle is
/// non-zero → allow.
#[test]
fn filter_allows_when_script_returns_nonzero_for_actual_source() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let (mut engine, robin_handle, sensitive_handle, _) = build_engine();
    // EventView is code 0.  Stimulus carries Robin as the Human
    // source — the stimulus info encodes a 0-based human handle, so
    // `filter_stimulus` will translate it to `robin_handle` before
    // passing to the script.
    let robin_human = crate::natives::ScriptHandleCodec::actor_handle_index(robin_handle)
        .expect("valid robin handle") as u32;
    let stim = crate::ai::Stimulus::with_human(crate::ai::StimulusType::EventView, robin_human);

    let allowed = engine.filter_stimulus(sim, &LevelAssets::new(), sensitive_handle, &stim);
    assert!(
        allowed,
        "non-zero source → script returns source → allow (got {allowed})"
    );
}

/// Same script, but stimulus carries no Human source → filter passes
/// `source=0` to the script → script returns 0 → block.  This is the
/// failure mode the old `source=0` precompute masked.
#[test]
fn filter_blocks_when_script_returns_zero_for_unknown_source() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let (mut engine, _, sensitive_handle, _) = build_engine();
    // `Stimulus::new` leaves `info = StimulusInfo::None`, so
    // `filter_stimulus` passes source=0.
    let stim = crate::ai::Stimulus::new(crate::ai::StimulusType::EventView);

    let allowed = engine.filter_stimulus(sim, &LevelAssets::new(), sensitive_handle, &stim);
    assert!(!allowed, "source=0 → script returns 0 → block");
}

/// Unmapped stimulus types still pass through `FilterAIEvent` with code -2,
/// matching the default switch arm in the original `StartThink`.
#[test]
fn filter_runs_for_unmapped_stimulus_type() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let (mut engine, _, sensitive_handle, _) = build_engine();
    // EventEnemyNear exists in the original but has no public AI event-code
    // mapping. The test script returns its source parameter, so source=0
    // proves the -2 path invoked the filter when it blocks the stimulus.
    let stim = crate::ai::Stimulus::new(crate::ai::StimulusType::EventEnemyNear);

    let allowed = engine.filter_stimulus(sim, &LevelAssets::new(), sensitive_handle, &stim);
    assert!(
        !allowed,
        "unmapped stimulus type must run FilterAIEvent(-2)"
    );
}

/// Actors with a bound script that doesn't override `FilterAIEvent` inherit
/// the base class's `return 1` default. The `actor_has_function` pre-check
/// distinguishes that optional missing method from a script-authored zero.
#[test]
fn filter_allows_when_actor_has_no_filter_override() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let (mut engine, _, _, noov_handle) = build_engine();
    let stim = crate::ai::Stimulus::new(crate::ai::StimulusType::EventView);

    let allowed = engine.filter_stimulus(sim, &LevelAssets::new(), noov_handle, &stim);
    assert!(
        allowed,
        "no FilterAIEvent override → base returns 1 → allow"
    );
}

/// Actors with no bound script instance at all pass through
/// unfiltered.  (Most shipped actors aren't scripted.)
#[test]
fn filter_allows_when_actor_not_bound_to_any_script() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    let script = MissionScript::from_scb(build_scb()).expect("mission script builds");
    engine.scripts.mission = Some(script);

    let unbound_id = engine.add_entity(make_scripted_soldier("SourceSensitive"));
    let unbound_handle = crate::natives::ScriptHandleCodec::actor_handle(unbound_id);

    let stim = crate::ai::Stimulus::new(crate::ai::StimulusType::EventView);
    assert!(
        engine.filter_stimulus(sim, &LevelAssets::new(), unbound_handle, &stim),
        "no bound script → allow"
    );
}

/// `dispatch_filtered_stimulus` should skip `think()` entirely when
/// the filter blocks, and should return `false` to the caller.
#[test]
fn dispatch_returns_false_when_filter_blocks_and_skips_think() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let (mut engine, _, sensitive_handle, _) = build_engine();

    // Snapshot AI state pre-dispatch.
    let sensitive_idx = crate::natives::ScriptHandleCodec::actor_handle_index(sensitive_handle)
        .expect("valid test handle");
    let sensitive_entity_id = EntityId::Pc(crate::entity_id::PcId(sensitive_idx as u32));
    let before_state = engine
        .world
        .entities
        .get(sensitive_entity_id)
        .and_then(|e| e.ai_controller())
        .map(|ai| (ai.current_state, ai.current_substate));

    // EventView with no human info → source=0 → script blocks.
    let stim = crate::ai::Stimulus::new(crate::ai::StimulusType::EventView);
    let ctx = crate::ai::AiContext::default();
    let tick_data = crate::ai::AiPerTickData::stub();

    let handled = engine.dispatch_filtered_stimulus(
        sim,
        &LevelAssets::new(),
        sensitive_entity_id,
        &stim,
        &ctx,
        &tick_data,
    );
    assert!(!handled, "filter blocked → dispatch returns false");

    // State should be unchanged (think() never ran).
    let after_state = engine
        .world
        .entities
        .get(sensitive_entity_id)
        .and_then(|e| e.ai_controller())
        .map(|ai| (ai.current_state, ai.current_substate));
    assert_eq!(
        before_state, after_state,
        "think() must not run when filter blocks"
    );
}

// ───────── Nested-native VM dispatch ─────────
//
// Tests for the `PrototypeFilterEvent` native re-entering the script
// subsystem mid-execution.  The native calls
// `prototype.FilterAIEvent(source, event)` from inside a running
// script — implemented via a yield-and-resume pipeline
// (`StopReason::Yield` and the sole `EngineInner` callback driver).

fn q_aff0_iconstant(dst: u16, constant: i32) -> Quad {
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

fn q_aff1_native_get_return(dst: u16) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&dst.to_le_bytes());
    Quad {
        operation: Opcode::Aff1NativeGetReturn as u8,
        operands: ops,
    }
}

fn q_iadd(dst: u16, a: u16, b: u16) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&dst.to_le_bytes());
    ops[2..4].copy_from_slice(&a.to_le_bytes());
    ops[4..6].copy_from_slice(&b.to_le_bytes());
    Quad {
        operation: Opcode::Aff2IAdd as u8,
        operands: ops,
    }
}

fn q_ieq(dst: u16, a: u16, b: u16) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&dst.to_le_bytes());
    ops[2..4].copy_from_slice(&a.to_le_bytes());
    ops[4..6].copy_from_slice(&b.to_le_bytes());
    Quad {
        operation: Opcode::Aff2IEq as u8,
        operands: ops,
    }
}

fn q_if_not_zero_goto(sym: u16, addr: u32) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&sym.to_le_bytes());
    ops[4..8].copy_from_slice(&addr.to_le_bytes());
    Quad {
        operation: Opcode::IfNotZeroGoto as u8,
        operands: ops,
    }
}

const TMP1: u16 = 0xC004;
const TMP2: u16 = 0xC008;
const TMP3: u16 = 0xC00C;
const TMP4: u16 = 0xC010;
const TMP5: u16 = 0xC014;

/// Build an SCB with two classes:
///  - `OuterCaller::FilterAIEvent(prototype, source, event)` invokes
///    the `PrototypeFilterEvent` native and returns its result.
///  - `InnerTarget::FilterAIEvent(...)` returns the constant `42`.
fn build_nested_scb() -> ScbFile {
    build_nested_scb_with_inner_native(None)
}

fn build_nested_scb_with_default_inner() -> ScbFile {
    let mut scb = build_nested_scb();
    let mut quads = Vec::new();
    let mut functions = Vec::new();
    for name in [
        "Initialize",
        "ActionChange",
        "HandleEvent",
        "ProcessMessage",
    ] {
        let base = quads.len() as i32;
        let (function, body) = stub_fn(name, base);
        functions.push(function);
        quads.extend(body);
    }
    scb.classes.push(ClassEntry {
        source_file: "test.scs".into(),
        class_name: "DefaultInnerTarget".into(),
        size_of_member_variables: 0,
        member_variables: vec![],
        functions,
        quads,
    });
    scb
}

fn build_nested_scb_with_inner_this(inner_returns_this: bool) -> ScbFile {
    build_nested_scb_with_inner_native(
        inner_returns_this.then_some(crate::natives::NativeFn::ThisActor),
    )
}

fn build_nested_scb_with_inner_native(inner_native: Option<crate::natives::NativeFn>) -> ScbFile {
    // Outer class.  FilterAIEvent reads the three params it was
    // called with, pushes them onto the native stack in order, calls
    // PrototypeFilterEvent, then returns whatever the native handed
    // back via Aff1NativeGetReturn.
    let mut outer_quads = Vec::new();
    let mut outer_functions = Vec::new();
    for name in [
        "Initialize",
        "ActionChange",
        "HandleEvent",
        "ProcessMessage",
    ] {
        let base = outer_quads.len() as i32;
        let (f, q) = stub_fn(name, base);
        outer_functions.push(f);
        outer_quads.extend(q);
    }
    let filter_addr = outer_quads.len() as i32;
    outer_functions.push(Function {
        name: "FilterAIEvent".into(),
        address: filter_addr,
        num_parameters: 3,
        size_of_return_value: 4,
        size_of_parameters: 12,
        size_of_volatile: 0,
        size_of_temporary: 12,
    });
    // Three temporaries TMP0 / TMP1 / TMP2 hold the three inbound
    // params before they're pushed onto the native stack.
    outer_quads.push(q_begin_function(0, 3));
    outer_quads.push(q_aff1_get_param(TMP0, 0)); // prototype handle
    outer_quads.push(q_aff1_get_param(TMP1, 4)); // source
    outer_quads.push(q_aff1_get_param(TMP2, 8)); // event
    outer_quads.push(q_native_param(TMP0));
    outer_quads.push(q_native_param(TMP1));
    outer_quads.push(q_native_param(TMP2));
    outer_quads.push(q_native_call(
        crate::natives::NativeFn::PrototypeFilterEvent as u32,
    ));
    outer_quads.push(q_aff1_native_get_return(TMP0));
    outer_quads.push(q_return_val(TMP0));
    outer_quads.push(q_end_function());

    let outer_class = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "OuterCaller".into(),
        size_of_member_variables: 0,
        member_variables: vec![],
        functions: outer_functions,
        quads: outer_quads,
    };

    // Inner class. FilterAIEvent normally returns 42 as a recognisable
    // sentinel. The parity variant returns ThisActor so the test can prove
    // PrototypeFilterEvent preserves the outer receiver.
    let mut inner_quads = Vec::new();
    let mut inner_functions = Vec::new();
    for name in [
        "Initialize",
        "ActionChange",
        "HandleEvent",
        "ProcessMessage",
    ] {
        let base = inner_quads.len() as i32;
        let (f, q) = stub_fn(name, base);
        inner_functions.push(f);
        inner_quads.extend(q);
    }
    let inner_filter_addr = inner_quads.len() as i32;
    inner_functions.push(Function {
        name: "FilterAIEvent".into(),
        address: inner_filter_addr,
        num_parameters: 2,
        size_of_return_value: 4,
        size_of_parameters: 8,
        size_of_volatile: 0,
        size_of_temporary: 4,
    });
    inner_quads.push(q_begin_function(0, 1));
    if let Some(native) = inner_native {
        inner_quads.push(q_native_call(native as u32));
        inner_quads.push(q_aff1_native_get_return(TMP0));
    } else {
        inner_quads.push(q_aff0_iconstant(TMP0, 42));
    }
    inner_quads.push(q_return_val(TMP0));
    inner_quads.push(q_end_function());

    let inner_class = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "InnerTarget".into(),
        size_of_member_variables: 0,
        member_variables: vec![],
        functions: inner_functions,
        quads: inner_quads,
    };

    let startup = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: vec![],
        functions: vec![],
        quads: vec![],
    };

    ScbFile {
        version: crate::scb::SCB_VERSION,
        classes: vec![startup, outer_class, inner_class],
    }
}

/// Build one actor class whose filter calls the prototype passed as its first
/// parameter. `ThisActor` becomes the nested call's source, so binding the
/// class to A and B and starting A with prototype B produces B -> A -> A ...
/// until the explicit nested-call limit supplies the inherited allow result.
fn build_recursive_nested_scb() -> ScbFile {
    let mut scb = build_nested_scb();
    scb.classes
        .retain(|class| class.class_name != "InnerTarget");
    let recursive = scb
        .classes
        .iter_mut()
        .find(|class| class.class_name == "OuterCaller")
        .expect("nested fixture has OuterCaller");
    recursive.class_name = "RecursiveCaller".into();
    let filter = recursive
        .functions
        .iter_mut()
        .find(|function| function.name == "FilterAIEvent")
        .expect("recursive class has FilterAIEvent");
    filter.num_parameters = 2;
    filter.size_of_parameters = 8;
    filter.size_of_temporary = 12;
    recursive.quads.truncate(filter.address as usize);
    recursive.quads.extend([
        q_begin_function(0, 3),
        q_aff1_get_param(TMP0, 0),
        q_native_call(crate::natives::NativeFn::ThisActor as u32),
        q_aff1_native_get_return(TMP1),
        q_aff1_get_param(TMP2, 4),
        q_native_param(TMP0),
        q_native_param(TMP1),
        q_native_param(TMP2),
        q_native_call(crate::natives::NativeFn::PrototypeFilterEvent as u32),
        q_aff1_native_get_return(TMP0),
        q_return_val(TMP0),
        q_end_function(),
    ]);
    scb
}

/// Variant where the outer callback writes an NPC custom value before
/// yielding through `PrototypeFilterEvent`, and the nested callback reads the
/// same entity value. This pins the shared live-state requirement across VM
/// resume boundaries before ScriptEffects is structurally removed.
fn build_nested_entity_mutation_scb() -> ScbFile {
    let mut scb = build_nested_scb();

    let outer = scb
        .classes
        .iter_mut()
        .find(|class| class.class_name == "OuterCaller")
        .expect("nested fixture has OuterCaller");
    let outer_filter = outer
        .functions
        .iter_mut()
        .find(|function| function.name == "FilterAIEvent")
        .expect("OuterCaller has FilterAIEvent");
    outer_filter.size_of_temporary = 12;
    outer.quads.truncate(outer_filter.address as usize);
    outer.quads.extend([
        q_begin_function(0, 3),
        q_aff1_get_param(TMP0, 0),
        q_aff0_iconstant(TMP1, 3),
        q_aff0_iconstant(TMP2, 77),
        q_native_param(TMP0),
        q_native_param(TMP1),
        q_native_param(TMP2),
        q_native_call(crate::natives::NativeFn::SetCustomNPCValue as u32),
        q_aff1_get_param(TMP0, 0),
        q_aff1_get_param(TMP1, 4),
        q_aff1_get_param(TMP2, 8),
        q_native_param(TMP0),
        q_native_param(TMP1),
        q_native_param(TMP2),
        q_native_call(crate::natives::NativeFn::PrototypeFilterEvent as u32),
        q_aff1_native_get_return(TMP0),
        q_return_val(TMP0),
        q_end_function(),
    ]);

    let inner = scb
        .classes
        .iter_mut()
        .find(|class| class.class_name == "InnerTarget")
        .expect("nested fixture has InnerTarget");
    let inner_filter = inner
        .functions
        .iter_mut()
        .find(|function| function.name == "FilterAIEvent")
        .expect("InnerTarget has FilterAIEvent");
    inner_filter.size_of_temporary = 8;
    inner.quads.truncate(inner_filter.address as usize);
    inner.quads.extend([
        q_begin_function(0, 2),
        q_aff1_get_param(TMP0, 0),
        q_aff0_iconstant(TMP1, 3),
        q_native_param(TMP0),
        q_native_param(TMP1),
        q_native_call(crate::natives::NativeFn::GetCustomNPCValue as u32),
        q_aff1_native_get_return(TMP0),
        q_return_val(TMP0),
        q_end_function(),
    ]);

    scb
}

/// Variant where the outer callback adds a canonical AI repulsive point and
/// passes its generated id through `PrototypeFilterEvent`; the nested callback
/// deletes that same point before the outer VM resumes.
fn build_nested_ai_global_mutation_scb() -> ScbFile {
    let mut scb = build_nested_scb();

    let outer = scb
        .classes
        .iter_mut()
        .find(|class| class.class_name == "OuterCaller")
        .expect("nested fixture has OuterCaller");
    let outer_filter = outer
        .functions
        .iter_mut()
        .find(|function| function.name == "FilterAIEvent")
        .expect("OuterCaller has FilterAIEvent");
    outer_filter.size_of_temporary = 20;
    outer.quads.truncate(outer_filter.address as usize);
    outer.quads.extend([
        q_begin_function(0, 5),
        q_aff1_get_param(TMP0, 0),
        q_aff0_iconstant(
            TMP1,
            crate::natives::ScriptHandleCodec::location_handle_from_index(0),
        ),
        q_aff0_iconstant(TMP2, 10.0_f32.to_bits() as i32),
        q_aff0_iconstant(TMP3, 20.0_f32.to_bits() as i32),
        q_aff0_iconstant(TMP4, 0),
        q_native_param(TMP1),
        q_native_param(TMP2),
        q_native_param(TMP3),
        q_native_param(TMP4),
        q_native_call(crate::natives::NativeFn::AddRepulsivePoint as u32),
        q_aff1_native_get_return(TMP4),
        q_aff1_get_param(TMP1, 4),
        q_native_param(TMP0),
        q_native_param(TMP1),
        q_native_param(TMP4),
        q_native_call(crate::natives::NativeFn::PrototypeFilterEvent as u32),
        q_aff1_native_get_return(TMP0),
        q_return_val(TMP0),
        q_end_function(),
    ]);

    let inner = scb
        .classes
        .iter_mut()
        .find(|class| class.class_name == "InnerTarget")
        .expect("nested fixture has InnerTarget");
    let inner_filter = inner
        .functions
        .iter_mut()
        .find(|function| function.name == "FilterAIEvent")
        .expect("InnerTarget has FilterAIEvent");
    inner_filter.size_of_temporary = 4;
    inner.quads.truncate(inner_filter.address as usize);
    inner.quads.extend([
        q_begin_function(0, 1),
        q_aff1_get_param(TMP0, 4),
        q_native_param(TMP0),
        q_native_call(crate::natives::NativeFn::DeleteRepulsivePoint as u32),
        q_aff0_iconstant(TMP0, 42),
        q_return_val(TMP0),
        q_end_function(),
    ]);

    scb
}

#[test]
fn nested_callback_keeps_the_canonical_query_views() {
    let scb =
        build_nested_scb_with_inner_native(Some(crate::natives::NativeFn::GetNumberOfSelectedPCs));
    let outer_handle = 11;
    let inner_handle = 22;
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(MissionScript::from_scb(scb).expect("scb builds"));
    engine.players.seats[0].selection = vec![
        crate::element::EntityId::Pc(crate::entity_id::PcId(0)),
        crate::element::EntityId::Pc(crate::entity_id::PcId(1)),
        crate::element::EntityId::Pc(crate::entity_id::PcId(2)),
    ];
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    engine
        .with_script_session(
            &crate::sim_rng::test_context(),
            &assets,
            |script, domains, capabilities| {
                assert!(script.bind_actor(outer_handle, "OuterCaller", domains, capabilities));
                assert!(script.bind_actor(inner_handle, "InnerTarget", domains, capabilities));
            },
        )
        .expect("mission installed");
    let result = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(outer_handle),
            "FilterAIEvent",
            &[inner_handle, 0, 0],
            crate::natives::ScriptCallFrame::actor(outer_handle),
        )
        .expect("nested dispatch runs cleanly");

    assert_eq!(result, 3, "the inner native sees canonical selection state");
}

#[test]
fn ordinary_actor_callback_binds_this_to_the_target_actor() {
    let scb = build_nested_scb_with_inner_this(true);
    let mut script = MissionScript::from_scb(scb).expect("scb builds");
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut entity_store = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut entity_store,
        &mut ai_global,
        &mut fast_grid,
    );
    let inner_handle = 22;
    assert!(script.bind_actor(
        inner_handle,
        "InnerTarget",
        &mut script_domains,
        &capabilities,
    ));

    drop(capabilities);
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(script);
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    let result = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(inner_handle),
            "FilterAIEvent",
            &[0, 0],
            crate::natives::ScriptCallFrame::actor(inner_handle),
        )
        .expect("direct actor callback runs cleanly");

    assert_eq!(result, inner_handle);
    assert_eq!(
        engine
            .scripts
            .mission
            .as_ref()
            .unwrap()
            .active_call_frame_count(),
        0,
        "call context is restored"
    );
}

#[test]
fn scroll_callback_binds_this_scroll_and_unwinds_the_frame() {
    let scb = build_nested_scb_with_inner_native(Some(crate::natives::NativeFn::ThisScroll));
    let mut script = MissionScript::from_scb(scb).expect("scb builds");
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut entity_store = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut entity_store,
        &mut ai_global,
        &mut fast_grid,
    );
    let scroll_handle = 23;
    assert!(script.bind_scroll(
        scroll_handle,
        "InnerTarget",
        &mut script_domains,
        &capabilities,
    ));

    drop(capabilities);
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(script);
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    let frame = crate::natives::ScriptCallFrame::default()
        .with_script_this(scroll_handle)
        .with_current_scroll(scroll_handle);
    let result = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Scroll(scroll_handle),
            "FilterAIEvent",
            &[0, 0],
            frame,
        )
        .expect("scroll callback runs cleanly");

    assert_eq!(result, scroll_handle);
    assert_eq!(
        engine
            .scripts
            .mission
            .as_ref()
            .unwrap()
            .active_call_frame_count(),
        0
    );
}

#[test]
fn prototype_filter_event_preserves_the_outer_this_actor() {
    // Original: RHScript::PrototypeFilterEvent deliberately leaves
    // pScriptThis unchanged so the prototype knows the actual event receiver.
    // See original-code/RHScript.cpp:6508-6535.
    let scb = build_nested_scb_with_inner_this(true);
    let mut script = MissionScript::from_scb(scb).expect("scb builds");
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut entity_store = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut entity_store,
        &mut ai_global,
        &mut fast_grid,
    );
    let outer_handle = 11;
    let prototype_handle = 22;
    assert!(script.bind_actor(
        outer_handle,
        "OuterCaller",
        &mut script_domains,
        &capabilities,
    ));
    assert!(script.bind_actor(
        prototype_handle,
        "InnerTarget",
        &mut script_domains,
        &capabilities,
    ));

    drop(capabilities);
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(script);
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    let result = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(outer_handle),
            "FilterAIEvent",
            &[prototype_handle, 0, 0],
            crate::natives::ScriptCallFrame::actor(outer_handle),
        )
        .expect("nested prototype dispatch runs cleanly");

    assert_eq!(
        result, outer_handle,
        "the prototype's ThisActor must remain the outer event receiver"
    );
    assert_eq!(
        engine
            .scripts
            .mission
            .as_ref()
            .unwrap()
            .active_call_frame_count(),
        0,
        "call context is restored"
    );
}

/// Smoke test: an actor script's `FilterAIEvent` calls
/// `PrototypeFilterEvent` on a sibling actor, and the nested call's
/// return value flows back into the outer return.  Verifies that:
///
///  1. The native arm returns an explicit script-call yield.
///  2. The interpreter carries it in `StopReason::Yield`.
///  3. The shared engine driver dispatches the queued call against the target
///     actor's bound script.
///  4. The result (`42`) is patched into the outer VM's
///     `native_return_value` and read by `Aff1NativeGetReturn`.
///  5. The outer VM resumes and returns the resolved sentinel.
#[test]
fn prototype_filter_event_dispatches_to_target_actor_script() {
    let scb = build_nested_scb();
    let mut script = MissionScript::from_scb(scb).expect("scb builds");
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut entity_store = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut entity_store,
        &mut ai_global,
        &mut fast_grid,
    );

    // Bind two synthetic actor instances. Their entity handles don't need to
    // map to real engine entities because these scripts never invoke an
    // entity-lookup native.
    let outer_handle = 1;
    let inner_handle = 2;
    assert!(script.bind_actor(
        outer_handle,
        "OuterCaller",
        &mut script_domains,
        &capabilities,
    ));
    assert!(script.bind_actor(
        inner_handle,
        "InnerTarget",
        &mut script_domains,
        &capabilities,
    ));

    drop(capabilities);
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(script);
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    let result = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(outer_handle),
            "FilterAIEvent",
            &[inner_handle, 0, 0],
            crate::natives::ScriptCallFrame::actor(outer_handle),
        )
        .expect("nested dispatch runs cleanly");

    assert_eq!(
        result, 42,
        "outer FilterAIEvent should return the inner target's sentinel \
         (42), proving the nested PrototypeFilterEvent dispatch fired"
    );
}

#[test]
fn recursive_prototype_filter_event_stops_at_call_stack_limit() {
    let mut script =
        MissionScript::from_scb(build_recursive_nested_scb()).expect("recursive SCB builds");
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut entity_store = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut entity_store,
        &mut ai_global,
        &mut fast_grid,
    );
    let actor_a = 1;
    let actor_b = 2;
    assert!(script.bind_actor(
        actor_a,
        "RecursiveCaller",
        &mut script_domains,
        &capabilities,
    ));
    assert!(script.bind_actor(
        actor_b,
        "RecursiveCaller",
        &mut script_domains,
        &capabilities,
    ));

    drop(capabilities);
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(script);
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    let error = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(actor_a),
            "FilterAIEvent",
            &[actor_b, 0],
            crate::natives::ScriptCallFrame::actor(actor_a),
        )
        .expect_err("recursive nested dispatch reaches the explicit limit");

    assert!(error.contains("depth limit"));
    assert_eq!(
        engine
            .scripts
            .mission
            .as_ref()
            .unwrap()
            .active_call_frame_count(),
        0
    );
}

#[test]
fn script_session_preserves_nested_pending_call_resume_and_restoration() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let script = MissionScript::from_scb(build_nested_scb()).expect("scb builds");
    let outer_handle = 1;
    let inner_handle = 2;
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    engine.scripts.mission = Some(script);
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);

    engine
        .with_script_session(sim, &assets, |script, script_domains, capabilities| {
            assert!(script.bind_actor(outer_handle, "OuterCaller", script_domains, capabilities));
            assert!(script.bind_actor(inner_handle, "InnerTarget", script_domains, capabilities));
        })
        .expect("mission script stays present");

    let result = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(outer_handle),
            "FilterAIEvent",
            &[inner_handle, 0, 0],
            crate::natives::ScriptCallFrame::actor(outer_handle),
        )
        .expect("nested dispatch runs cleanly");

    assert_eq!(
        result, 42,
        "the nested return register reaches the outer VM"
    );
    let script = engine.scripts.mission.as_ref().unwrap();
    assert_eq!(script.active_call_frame_count(), 0);
}

#[test]
fn prototype_filter_event_missing_override_uses_actor_base_default() {
    let scb = build_nested_scb_with_default_inner();
    let mut script = MissionScript::from_scb(scb).expect("scb builds");
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut entity_store = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut entity_store,
        &mut ai_global,
        &mut fast_grid,
    );
    let outer_handle = 1;
    let prototype_handle = 2;
    assert!(script.bind_actor(
        outer_handle,
        "OuterCaller",
        &mut script_domains,
        &capabilities,
    ));
    assert!(script.bind_actor(
        prototype_handle,
        "DefaultInnerTarget",
        &mut script_domains,
        &capabilities,
    ));

    drop(capabilities);
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(script);
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    let result = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(outer_handle),
            "FilterAIEvent",
            &[prototype_handle, 0, 0],
            crate::natives::ScriptCallFrame::actor(outer_handle),
        )
        .expect("nested dispatch runs cleanly");

    assert_eq!(
        result, 1,
        "a bound actor without an override inherits FilterAIEvent's allow result"
    );
}

#[test]
fn nested_prototype_callback_observes_outer_native_entity_mutation() {
    let scb = build_nested_entity_mutation_scb();
    let mut script = MissionScript::from_scb(scb).expect("scb builds");
    let mut script_domains = crate::engine::ScriptDomains::default();
    let outer_handle = crate::natives::ScriptHandleCodec::actor_handle_from_index(0);
    let prototype_handle = crate::natives::ScriptHandleCodec::actor_handle_from_index(1);
    let mut entity_store = crate::entities::Entities::from_legacy_slots(vec![
        Some(make_scripted_soldier("OuterCaller")),
        Some(make_scripted_soldier("InnerTarget")),
    ]);
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut entity_store,
        &mut ai_global,
        &mut fast_grid,
    );
    assert!(script.bind_actor(
        outer_handle,
        "OuterCaller",
        &mut script_domains,
        &capabilities,
    ));
    assert!(script.bind_actor(
        prototype_handle,
        "InnerTarget",
        &mut script_domains,
        &capabilities,
    ));

    drop(capabilities);
    let mut engine = EngineInner::new();
    engine.world.entities = entity_store;
    engine.ai.global = ai_global;
    engine.world.fast_grid = fast_grid;
    engine.scripts.mission = Some(script);
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    let result = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(outer_handle),
            "FilterAIEvent",
            &[prototype_handle, prototype_handle, 0],
            crate::natives::ScriptCallFrame::actor(outer_handle),
        )
        .expect("nested dispatch runs cleanly");

    assert_eq!(
        result, 77,
        "the nested VM must read the entity mutation made before the outer VM yielded"
    );
    assert_eq!(
        engine
            .world
            .entities
            .get_legacy_slot(1)
            .expect("prototype entity remains installed")
            .1
            .npc_data()
            .expect("prototype is an NPC")
            .custom_values[3],
        77
    );
}

#[test]
fn nested_prototype_callback_observes_canonical_ai_global_mutation() {
    let sim_context = crate::sim_rng::test_context();
    let mut script = MissionScript::from_scb(build_nested_ai_global_mutation_scb())
        .expect("nested AI-global SCB builds");
    let mut script_domains = crate::engine::ScriptDomains::default();
    let outer_handle = 1;
    let prototype_handle = 2;
    let mut entities = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim_context,
        &mut entities,
        &mut ai_global,
        &mut fast_grid,
    );
    assert!(script.bind_actor(
        outer_handle,
        "OuterCaller",
        &mut script_domains,
        &capabilities,
    ));
    assert!(script.bind_actor(
        prototype_handle,
        "InnerTarget",
        &mut script_domains,
        &capabilities,
    ));
    drop(capabilities);

    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    engine.scripts.mission = Some(script);
    let mut assets = LevelAssets::new();
    assets.scripts.location_count = 1;
    assets.scripts.point_count = 1;
    assets.scripts.location_positions = std::sync::Arc::new(vec![(12.0, 34.0)]);
    assets.scripts.location_layers = std::sync::Arc::new(vec![2]);
    assets.scripts.location_sectors = std::sync::Arc::new(vec![44]);
    engine.attach_script_bindings(&assets);

    let result = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(outer_handle),
            "FilterAIEvent",
            &[prototype_handle, 0, 0],
            crate::natives::ScriptCallFrame::actor(outer_handle),
        )
        .expect("nested AI-global dispatch runs cleanly");

    assert_eq!(result, 42);
    assert_eq!(engine.ai.global.next_repulsive_point_id, 2);
    assert!(
        engine.ai.global.repulsive_points.is_empty(),
        "the nested callback must delete the point added to canonical AI state before outer resume"
    );
}

/// A prototype with no bound Rust script instance takes the original actor
/// base-class `FilterAIEvent` result: one (allow). `RHScript` forwards the call
/// directly to the NPC (`RHScript.cpp:6519-6537`), and the shipped ActorScript
/// base implementation returns one rather than synthesizing a blocked event.
#[test]
fn prototype_filter_event_unbound_target_is_a_required_vm_error() {
    let scb = build_nested_scb();
    let mut script = MissionScript::from_scb(scb).expect("scb builds");
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut entity_store = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut entity_store,
        &mut ai_global,
        &mut fast_grid,
    );

    let outer_handle = 1;
    assert!(script.bind_actor(
        outer_handle,
        "OuterCaller",
        &mut script_domains,
        &capabilities,
    ));
    // Note: don't bind anyone for handle 99.

    drop(capabilities);
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(script);
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    let error = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(outer_handle),
            "FilterAIEvent",
            &[99, 0, 0],
            crate::natives::ScriptCallFrame::actor(outer_handle),
        )
        .expect_err("the native referenced a missing required target VM");

    assert!(error.contains("required VM is not bound"));
}

// ───────── Creation-ordered ActionChange dispatch ─────────

fn action_change_class(class_name: &str, temporary_count: u16, body: Vec<Quad>) -> ClassEntry {
    let mut quads = vec![q_begin_function(0, temporary_count)];
    quads.extend(body);
    quads.extend([q_return(), q_end_function()]);
    ClassEntry {
        source_file: "action_change_test.scs".into(),
        class_name: class_name.into(),
        size_of_member_variables: 0,
        member_variables: vec![],
        functions: vec![Function {
            name: "ActionChange".into(),
            address: 0,
            num_parameters: 2,
            size_of_return_value: 0,
            size_of_parameters: 8,
            size_of_volatile: 0,
            size_of_temporary: i32::from(temporary_count) * 4,
        }],
        quads,
    }
}

fn action_change_record_body(actor_handle: i32) -> Vec<Quad> {
    vec![
        q_aff1_get_param(TMP0, 0),
        q_aff1_get_param(TMP1, 4),
        q_aff0_iconstant(TMP2, actor_handle),
        q_aff0_iconstant(TMP3, 0),
        q_native_param(TMP2),
        q_native_param(TMP3),
        q_native_param(TMP0),
        q_native_call(crate::natives::NativeFn::SetCustomNPCValue as u32),
        q_aff0_iconstant(TMP3, 1),
        q_native_param(TMP2),
        q_native_param(TMP3),
        q_native_param(TMP1),
        q_native_call(crate::natives::NativeFn::SetCustomNPCValue as u32),
    ]
}

fn build_action_change_scb(target_handle: i32, observer_handle: i32) -> ScbFile {
    let mut self_mutator = action_change_record_body(observer_handle);
    self_mutator.extend([
        q_aff0_iconstant(TMP3, 10),
        q_native_param(TMP2),
        q_native_param(TMP3),
        q_native_call(crate::natives::NativeFn::SetActorPosture as u32),
    ]);

    ScbFile {
        version: crate::scb::SCB_VERSION,
        classes: vec![
            ClassEntry {
                source_file: "action_change_test.scs".into(),
                class_name: "StartUp".into(),
                size_of_member_variables: 0,
                member_variables: vec![],
                functions: vec![],
                quads: vec![],
            },
            action_change_class(
                "PostureMutator",
                2,
                vec![
                    q_aff0_iconstant(TMP0, target_handle),
                    q_aff0_iconstant(TMP1, 10),
                    q_native_param(TMP0),
                    q_native_param(TMP1),
                    q_native_call(crate::natives::NativeFn::SetActorPosture as u32),
                ],
            ),
            action_change_class(
                "ActionObserver",
                4,
                action_change_record_body(observer_handle),
            ),
            action_change_class("SelfPostureMutator", 4, self_mutator),
        ],
    }
}

fn install_test_action(
    engine: &mut EngineInner,
    actor: EntityId,
    action: crate::order::OrderType,
    old_action: crate::order::OrderType,
) {
    engine
        .world
        .entities
        .get_mut(actor)
        .expect("action-change fixture actor exists")
        .actor_data_mut()
        .expect("action-change fixture entity is an actor")
        .old_action = old_action;
    let mut element =
        crate::sequence::SequenceElement::new(1, crate::element::Command::Wait, Some(actor));
    element.priority = crate::sequence::SequencePriority::Wait;
    element
        .orders
        .push_back(crate::order::Order::test_new(action, 0.0, 0.0));
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .take_pending_synchronous_actions()
            .len(),
        1,
        "the fixture consumes the synthetic wait element's initial Instruct"
    );
}

fn bind_test_actor_animations(
    engine: &mut EngineInner,
    actor: EntityId,
    actions: &[crate::order::OrderType],
) {
    let mut scripts = Vec::new();
    let mut conversion =
        vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
    for &action in actions {
        conversion[action as usize] = scripts.len() as u16;
        let script = crate::sprite_script::SpriteScript {
            action_id: action as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2, 3],
            delays: vec![0, 0, 0],
            distances: vec![0, 0, 0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0, 0, 0],
        };
        scripts.extend(std::iter::repeat_n(script, 16));
    }
    engine
        .world
        .entities
        .get_mut(actor)
        .expect("actor-animation fixture actor exists")
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(scripts),
        std::sync::Arc::new(conversion),
    );
}

fn install_test_order_queue(
    engine: &mut EngineInner,
    actor: EntityId,
    orders: impl IntoIterator<Item = crate::order::Order>,
) -> crate::sequence::SequenceId {
    let mut element =
        crate::sequence::SequenceElement::new(1, crate::element::Command::PlayAnim, Some(actor));
    for order in orders {
        element.orders.push_back(order);
    }
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    let _ = engine
        .orders
        .sequence_manager
        .take_pending_synchronous_actions();
    sequence
}

fn bind_action_observer(engine: &mut EngineInner, assets: &LevelAssets, actor: EntityId) {
    let handle = crate::natives::ScriptHandleCodec::actor_handle(actor);
    engine.scripts.mission = Some(
        MissionScript::from_scb(build_action_change_scb(handle, handle))
            .expect("action-observer SCB builds"),
    );
    engine.attach_script_bindings(assets);
    engine
        .with_script_session(
            &crate::sim_rng::test_context(),
            assets,
            |script, domains, capabilities| {
                assert!(script.bind_actor(handle, "ActionObserver", domains, capabilities));
            },
        )
        .expect("action-observer mission remains installed");
}

fn action_change_ordering_engine(
    mutator_before_observer: bool,
) -> (EngineInner, LevelAssets, EntityId, EntityId) {
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();

    let (first_class, second_class) = if mutator_before_observer {
        ("PostureMutator", "ActionObserver")
    } else {
        ("ActionObserver", "PostureMutator")
    };
    let first = engine.add_entity(make_scripted_soldier(first_class));
    let second = engine.add_entity(make_scripted_soldier(second_class));
    let (mutator, observer) = if mutator_before_observer {
        (first, second)
    } else {
        (second, first)
    };
    let mutator_handle = crate::natives::ScriptHandleCodec::actor_handle(mutator);
    let observer_handle = crate::natives::ScriptHandleCodec::actor_handle(observer);

    engine.scripts.mission = Some(
        MissionScript::from_scb(build_action_change_scb(observer_handle, observer_handle))
            .expect("action-change SCB builds"),
    );
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    engine
        .with_script_session(
            &crate::sim_rng::test_context(),
            &assets,
            |script, domains, capabilities| {
                assert!(
                    script.bind_actor(mutator_handle, "PostureMutator", domains, capabilities,)
                );
                assert!(script.bind_actor(
                    observer_handle,
                    "ActionObserver",
                    domains,
                    capabilities,
                ));
            },
        )
        .expect("action-change mission remains installed");

    install_test_action(
        &mut engine,
        mutator,
        crate::order::OrderType::RunningUpright,
        crate::order::OrderType::WaitingUpright,
    );
    install_test_action(
        &mut engine,
        observer,
        crate::order::OrderType::WalkingUpright,
        crate::order::OrderType::WaitingUpright,
    );
    (engine, assets, mutator, observer)
}

fn observed_action_args(engine: &EngineInner, actor: EntityId) -> (i32, i32) {
    let values = &engine
        .world
        .entities
        .get(actor)
        .expect("observed action actor remains installed")
        .npc_data()
        .expect("observed action actor remains an NPC")
        .custom_values;
    (values[0], values[1])
}

#[test]
fn action_change_unbound_nonempty_script_class_does_not_consume_transition() {
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    let actor = engine.add_entity(make_scripted_soldier("ActionObserver"));
    let handle = crate::natives::ScriptHandleCodec::actor_handle(actor);
    engine.scripts.mission = Some(
        MissionScript::from_scb(build_action_change_scb(handle, handle))
            .expect("unbound ActionChange SCB builds"),
    );
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    install_test_action(
        &mut engine,
        actor,
        crate::order::OrderType::WalkingUpright,
        crate::order::OrderType::WaitingUpright,
    );

    engine.dispatch_actor_action_changes(&crate::sim_rng::test_context(), &assets);

    let actor_data = engine
        .world
        .entities
        .get(actor)
        .expect("unbound action-change actor remains installed")
        .actor_data()
        .expect("unbound action-change actor remains typed");
    assert_eq!(
        actor_data.old_action,
        crate::order::OrderType::WaitingUpright,
        "a class name without an instantiated actor VM is not scripted and must not consume the transition"
    );
    assert_eq!(
        observed_action_args(&engine, actor),
        (0, 0),
        "the unbound actor callback must not execute"
    );
}

#[test]
fn action_change_earlier_callback_changes_later_actor_snapshot() {
    let (mut engine, assets, _mutator, observer) = action_change_ordering_engine(true);

    engine.dispatch_actor_action_changes(&crate::sim_rng::test_context(), &assets);

    assert_eq!(
        observed_action_args(&engine, observer),
        (
            crate::order::OrderType::WaitingCrouched as i32,
            crate::order::OrderType::WaitingUpright as i32,
        ),
        "the later actor must snapshot the animation installed by the earlier callback"
    );
}

#[test]
fn action_change_later_callback_mutation_waits_for_visited_actor_next_pass() {
    let (mut engine, assets, _mutator, observer) = action_change_ordering_engine(false);
    let sim = crate::sim_rng::test_context();

    engine.dispatch_actor_action_changes(&sim, &assets);
    assert_eq!(
        observed_action_args(&engine, observer),
        (
            crate::order::OrderType::WalkingUpright as i32,
            crate::order::OrderType::WaitingUpright as i32,
        ),
        "an already visited actor keeps its pre-mutation callback arguments"
    );

    engine.dispatch_actor_action_changes(&sim, &assets);
    assert_eq!(
        observed_action_args(&engine, observer),
        (
            crate::order::OrderType::WaitingCrouched as i32,
            crate::order::OrderType::WalkingUpright as i32,
        ),
        "the later mutation must become the visited actor's next-pass transition"
    );
}

#[test]
fn action_change_self_mutation_stores_live_post_callback_animation() {
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    let actor = engine.add_entity(make_scripted_soldier("SelfPostureMutator"));
    let handle = crate::natives::ScriptHandleCodec::actor_handle(actor);
    engine.scripts.mission = Some(
        MissionScript::from_scb(build_action_change_scb(handle, handle))
            .expect("self-mutation ActionChange SCB builds"),
    );
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    engine
        .with_script_session(
            &crate::sim_rng::test_context(),
            &assets,
            |script, domains, capabilities| {
                assert!(script.bind_actor(handle, "SelfPostureMutator", domains, capabilities,));
            },
        )
        .expect("self-mutation mission remains installed");
    install_test_action(
        &mut engine,
        actor,
        crate::order::OrderType::WalkingUpright,
        crate::order::OrderType::WaitingUpright,
    );

    engine.dispatch_actor_action_changes(&crate::sim_rng::test_context(), &assets);

    assert_eq!(
        observed_action_args(&engine, actor),
        (
            crate::order::OrderType::WalkingUpright as i32,
            crate::order::OrderType::WaitingUpright as i32,
        ),
        "callback arguments must remain the pre-callback animation snapshot"
    );
    assert_eq!(
        engine
            .world
            .entities
            .get(actor)
            .expect("self-mutating actor remains installed")
            .actor_data()
            .expect("self-mutating actor remains typed")
            .old_action,
        crate::order::OrderType::WaitingCrouched,
        "old_action must retain the post-callback live animation"
    );
}

#[test]
fn generic_animation_skip_does_not_skip_action_change() {
    use crate::element::ActionState;
    use crate::order::OrderType;

    for skip in [
        "global-frozen",
        "inactive",
        "execution-frozen",
        "moving",
        "dead",
        "unconscious",
        "active-melee",
        "active-shot",
    ] {
        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = crate::campaign::Campaign::default();
        let actor = engine.add_entity(make_scripted_soldier("ActionObserver"));
        let stale = engine.add_entity(make_scripted_soldier(""));
        engine.remove_entity(stale);
        let assets = LevelAssets::new();
        bind_action_observer(&mut engine, &assets, actor);
        bind_test_actor_animations(&mut engine, actor, &[OrderType::WalkingUpright]);
        install_test_action(
            &mut engine,
            actor,
            OrderType::WalkingUpright,
            OrderType::WaitingUpright,
        );
        let (seq_id, elem_idx) = engine
            .orders
            .sequence_manager
            .current_element_for_actor(actor)
            .expect("skipped actor has a current element");
        engine
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .expect("skipped actor element remains installed")
            .orders
            .front_mut()
            .expect("skipped actor has a current order")
            .antagonist = Some(stale);
        {
            let entity = engine
                .world
                .entities
                .get_mut(actor)
                .expect("skipped actor exists for stale-reference setup");
            entity
                .human_data_mut()
                .expect("skipped actor is human")
                .opponents
                .push(stale);
            entity
                .actor_data_mut()
                .expect("skipped actor is typed")
                .active_door_pass = Some(crate::element::ActiveDoorPass {
                door_index: crate::gate::DoorIndex(u32::MAX),
                direct: true,
                steps: std::collections::VecDeque::new(),
                triggers_fired: 0,
                current_action: OrderType::Invalid,
                current_reverse: false,
                saved_action_state: None,
            });
        }
        let last_action_before = engine
            .world
            .entities
            .get(actor)
            .expect("skipped actor exists before setup")
            .element_data()
            .sprite
            .last_action;
        if skip == "global-frozen" {
            engine.set_actors_frozen(true);
        } else {
            let entity = engine
                .world
                .entities
                .get_mut(actor)
                .expect("skipped actor exists");
            match skip {
                "inactive" => entity.element_data_mut().active = false,
                "execution-frozen" => {
                    entity
                        .actor_data_mut()
                        .expect("skipped actor is typed")
                        .execution_frozen = true;
                }
                "moving" => {
                    entity
                        .actor_data_mut()
                        .expect("skipped actor is typed")
                        .action_state = ActionState::Moving;
                }
                "dead" => {
                    entity
                        .npc_data_mut()
                        .expect("skipped actor is an NPC")
                        .life_points = 0;
                }
                "unconscious" => {
                    entity
                        .human_data_mut()
                        .expect("skipped actor is human")
                        .unconscious = true;
                }
                "active-melee" => {
                    entity
                        .actor_data_mut()
                        .expect("skipped actor is typed")
                        .active_melee = crate::movement::ActiveMelee::new(
                        stale,
                        crate::weapons::SwordStrike::default(),
                        Some(seq_id),
                        elem_idx,
                    );
                }
                "active-shot" => {
                    entity
                        .actor_data_mut()
                        .expect("skipped actor is typed")
                        .active_shot = crate::movement::ActiveShot {
                        sequence_id: Some(seq_id),
                        element_index: elem_idx,
                        target: Some(stale),
                        ..Default::default()
                    };
                }
                _ => unreachable!(),
            }
        }

        engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

        assert_eq!(
            observed_action_args(&engine, actor),
            (
                OrderType::WalkingUpright as i32,
                OrderType::WaitingUpright as i32,
            ),
            "{skip} actors still own the base-Hourglass ActionChange boundary"
        );
        assert_eq!(
            engine
                .world
                .entities
                .get(actor)
                .expect("skipped actor remains installed")
                .element_data()
                .sprite
                .last_action,
            last_action_before,
            "{skip} must skip generic sprite execution"
        );
    }
}

#[test]
fn movement_owned_token_skip_does_not_sample_stale_execute_inputs() {
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_scripted_soldier(""));
    let stale = engine.add_entity(make_scripted_soldier(""));
    engine.remove_entity(stale);
    {
        let entity = engine
            .world
            .entities
            .get_mut(actor)
            .expect("token-skip actor exists");
        entity
            .human_data_mut()
            .expect("token-skip actor is human")
            .opponents
            .push(stale);
        entity
            .actor_data_mut()
            .expect("token-skip actor is typed")
            .active_door_pass = Some(crate::element::ActiveDoorPass {
            door_index: crate::gate::DoorIndex(u32::MAX),
            direct: true,
            steps: std::collections::VecDeque::new(),
            triggers_fired: 0,
            current_action: OrderType::Invalid,
            current_reverse: false,
            saved_action_state: None,
        });
    }
    let order = crate::order::Order::new(
        OrderType::WalkingWithSword,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    )
    .with_antagonist(stale);
    let mut element =
        crate::sequence::SequenceElement::new(1, crate::element::Command::Move, Some(actor));
    element.orders.push_back(order);
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    let _ = engine
        .orders
        .sequence_manager
        .take_pending_synchronous_actions();

    let (injuries, outcomes) = engine.tick_actor_animation_for(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        actor,
    );

    assert!(injuries.is_empty());
    assert!(outcomes.seq_advance.is_empty());
    assert_eq!(
        engine
            .world
            .entities
            .get(actor)
            .expect("token-skip actor remains installed")
            .element_data()
            .sprite
            .last_action,
        OrderType::NonanimationEnd,
        "the movement-owned token must skip generic Execute without dereferencing stale inputs"
    );
}

#[test]
fn per_actor_wait_initialization_does_not_publish_later_wait_to_earlier_callback() {
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    let first = engine.add_entity(make_scripted_soldier("WaitProbe"));
    let later = engine.add_entity(make_scripted_soldier(""));
    let first_handle = crate::natives::ScriptHandleCodec::actor_handle(first);
    let later_handle = crate::natives::ScriptHandleCodec::actor_handle(later);
    let body = vec![
        q_aff0_iconstant(TMP0, later_handle),
        q_native_param(TMP0),
        q_native_call(crate::natives::NativeFn::GetCurrentAction as u32),
        q_aff1_native_get_return(TMP1),
        q_aff0_iconstant(TMP2, first_handle),
        q_aff0_iconstant(TMP3, 0),
        q_native_param(TMP2),
        q_native_param(TMP3),
        q_native_param(TMP1),
        q_native_call(crate::natives::NativeFn::SetCustomNPCValue as u32),
    ];
    engine.scripts.mission = Some(
        MissionScript::from_scb(ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![
                ClassEntry {
                    source_file: "wait_isolation_test.scs".into(),
                    class_name: "StartUp".into(),
                    size_of_member_variables: 0,
                    member_variables: vec![],
                    functions: vec![],
                    quads: vec![],
                },
                action_change_class("WaitProbe", 4, body),
            ],
        })
        .expect("wait-isolation SCB builds"),
    );
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    engine
        .with_script_session(
            &crate::sim_rng::test_context(),
            &assets,
            |script, domains, capabilities| {
                assert!(script.bind_actor(first_handle, "WaitProbe", domains, capabilities));
            },
        )
        .expect("wait-isolation mission remains installed");

    engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

    assert_eq!(
        engine
            .world
            .entities
            .get(first)
            .expect("first wait-probe actor remains installed")
            .npc_data()
            .expect("first wait-probe actor remains NPC")
            .custom_values[0],
        0,
        "the earlier callback must observe the later actor before that actor's lazy Wait slot"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(later)
            .is_some(),
        "the later actor must initialize and dispatch its Wait only upon reaching its own slot"
    );
}

#[test]
fn combat_injury_think_finishes_before_same_slot_action_change() {
    use super::tick::{ActorAnimationBoundaryPhase as Phase, capture_actor_animation_boundary};

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_scripted_soldier(""));
    engine
        .world
        .entities
        .get_mut(actor)
        .and_then(|entity| entity.npc_data_mut())
        .and_then(|npc| npc.ai_brain.enemy_mut())
        .expect("combat-injury fixture has enemy AI")
        .hth_weapon_id = 1;
    let mut assets = LevelAssets::new();
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    profiles
        .soldiers
        .push(crate::profiles::SoldierProfile::default());
    profiles
        .hth_weapons
        .push(crate::profiles::HtHWeaponProfile::default());
    bind_test_actor_animations(
        &mut engine,
        actor,
        &[crate::order::OrderType::StandingUpSword],
    );
    let order = crate::order::Order::new(
        crate::order::OrderType::StandingUpSword,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    );
    let order_id = order.order_id;
    install_test_order_queue(&mut engine, actor, [order]);
    {
        let sprite = &mut engine
            .world
            .entities
            .get_mut(actor)
            .expect("combat-injury actor exists for sprite priming")
            .element_data_mut()
            .sprite;
        sprite.last_processed_order_id = order_id.get();
        sprite.last_action = crate::order::OrderType::StandingUpSword;
        sprite.current_row = 0;
        sprite.current_frame = 1;
        sprite.frame_count = 0;
        sprite.action_done_frame = 1;
        sprite.action_done_counter = 0;
    }
    engine.dispatch_ai_stimulus(
        actor,
        crate::ai::Stimulus::new(crate::ai::StimulusType::EventTimer),
    );

    let (_, phases) = capture_actor_animation_boundary(|| {
        engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets)
    });

    let think = phases
        .iter()
        .position(|phase| *phase == Phase::CombatInjuryThink(actor))
        .expect("terminating combat injury synchronously enters Think");
    let action_change = phases
        .iter()
        .position(|phase| *phase == Phase::ActionChange(actor))
        .expect("same actor reaches ActionChange");
    let completion = phases
        .iter()
        .position(|phase| *phase == Phase::CompletionEffects(actor))
        .expect("same actor applies completion effects");
    assert!(
        think < completion && completion < action_change,
        "StandingUpSword must run combat Think before completion/DoNext and ActionChange: {phases:?}"
    );
    assert_eq!(
        engine
            .world
            .entities
            .get(actor)
            .and_then(|entity| entity.ai_controller())
            .expect("combat-injury actor retains AI")
            .outbox
            .detection
            .stimuli
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![crate::ai::StimulusType::EventTimer],
        "the synchronous combat Think must preserve the older unrelated FIFO entry"
    );
}

#[test]
fn earlier_action_change_replacement_is_animated_at_the_later_actor_slot() {
    let (mut engine, assets, mutator, observer) = action_change_ordering_engine(true);
    bind_test_actor_animations(
        &mut engine,
        mutator,
        &[
            crate::order::OrderType::RunningUpright,
            crate::order::OrderType::WaitingCrouched,
        ],
    );
    bind_test_actor_animations(
        &mut engine,
        observer,
        &[
            crate::order::OrderType::WalkingUpright,
            crate::order::OrderType::WaitingCrouched,
        ],
    );

    engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

    assert_eq!(
        engine
            .world
            .entities
            .get(observer)
            .expect("later observer remains installed")
            .element_data()
            .sprite
            .last_action,
        crate::order::OrderType::WaitingCrouched,
        "the later actor must execute the order synchronously installed by the earlier ActionChange"
    );
}

#[test]
fn later_action_change_replacement_defers_already_visited_actor_animation() {
    let (mut engine, assets, mutator, observer) = action_change_ordering_engine(false);
    bind_test_actor_animations(
        &mut engine,
        mutator,
        &[
            crate::order::OrderType::RunningUpright,
            crate::order::OrderType::WaitingCrouched,
        ],
    );
    bind_test_actor_animations(
        &mut engine,
        observer,
        &[
            crate::order::OrderType::WalkingUpright,
            crate::order::OrderType::WaitingCrouched,
        ],
    );
    let sim = crate::sim_rng::test_context();

    engine.tick_actor_animation_action_change_slots(&sim, &assets);
    assert_eq!(
        engine
            .world
            .entities
            .get(observer)
            .expect("earlier observer remains installed")
            .element_data()
            .sprite
            .last_action,
        crate::order::OrderType::WalkingUpright,
        "a later callback cannot retroactively replace animation at an already visited slot"
    );

    engine.tick_actor_animation_action_change_slots(&sim, &assets);
    assert_eq!(
        engine
            .world
            .entities
            .get(observer)
            .expect("earlier observer remains installed next pass")
            .element_data()
            .sprite
            .last_action,
        crate::order::OrderType::WaitingCrouched,
        "the replacement must execute when the actor reaches its next-pass slot"
    );
}

#[test]
fn terminating_animation_promotes_next_order_before_same_actor_action_change() {
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    let actor = engine.add_entity(make_scripted_soldier("ActionObserver"));
    let assets = LevelAssets::new();
    bind_action_observer(&mut engine, &assets, actor);
    bind_test_actor_animations(
        &mut engine,
        actor,
        &[OrderType::Pointing, OrderType::Searching],
    );
    engine
        .world
        .entities
        .get_mut(actor)
        .expect("terminating actor exists")
        .actor_data_mut()
        .expect("terminating actor is typed")
        .old_action = OrderType::Pointing;

    let first = crate::order::Order::new(
        OrderType::Pointing,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    );
    let first_order_id = first.order_id;
    let second = crate::order::Order::new(
        OrderType::Searching,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    );
    install_test_order_queue(&mut engine, actor, [first, second]);
    {
        let sprite = &mut engine
            .world
            .entities
            .get_mut(actor)
            .expect("terminating actor exists for sprite priming")
            .element_data_mut()
            .sprite;
        sprite.last_processed_order_id = first_order_id.get();
        sprite.last_action = OrderType::Pointing;
        sprite.current_row = 0;
        sprite.current_frame = 1;
        sprite.frame_count = 0;
        sprite.action_done_frame = 1;
        sprite.action_done_counter = 0;
    }

    engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

    assert_eq!(
        observed_action_args(&engine, actor),
        (OrderType::Searching as i32, OrderType::Pointing as i32),
        "ActionChange must receive the order promoted by same-actor TERMINATED handling"
    );
    assert_eq!(
        engine
            .world
            .entities
            .get(actor)
            .expect("terminating actor remains installed")
            .actor_data()
            .expect("terminating actor remains typed")
            .old_action,
        OrderType::Searching,
        "retention must store the promoted live order"
    );
}

fn waking_up_creation_order_engine(
    rescuer_before_target: bool,
) -> (EngineInner, LevelAssets, EntityId, EntityId) {
    use crate::combat::CONCUSSION_THRESHOLD;
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let first = engine.add_entity(make_scripted_soldier(""));
    let second = engine.add_entity(make_scripted_soldier(""));
    let (rescuer, target) = if rescuer_before_target {
        (first, second)
    } else {
        (second, first)
    };
    {
        let target_entity = engine
            .world
            .entities
            .get_mut(target)
            .expect("wake target exists");
        target_entity.element_data_mut().posture = Posture::Lying;
        target_entity
            .human_data_mut()
            .expect("wake target is human")
            .unconscious = true;
        target_entity
            .human_data_mut()
            .expect("wake target is human")
            .concussion_of_the_brain = CONCUSSION_THRESHOLD;
    }
    bind_test_actor_animations(&mut engine, rescuer, &[OrderType::WakingUp]);
    bind_test_actor_animations(
        &mut engine,
        target,
        &[OrderType::BeingUnconscious, OrderType::StandingUp],
    );

    let waking = crate::order::Order::new(
        OrderType::WakingUp,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    )
    .with_antagonist(target);
    let waking_id = waking.order_id;
    install_test_order_queue(&mut engine, rescuer, [waking]);
    let unconscious = crate::order::Order::new(
        OrderType::BeingUnconscious,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    );
    install_test_order_queue(&mut engine, target, [unconscious]);
    {
        let sprite = &mut engine
            .world
            .entities
            .get_mut(rescuer)
            .expect("rescuer exists for sprite priming")
            .element_data_mut()
            .sprite;
        sprite.last_processed_order_id = waking_id.get();
        sprite.last_action = OrderType::WakingUp;
        sprite.current_row = 0;
        sprite.current_frame = 0;
        sprite.frame_count = 0;
        sprite.action_done_frame = 1;
        sprite.action_done_counter = 0;
    }
    (engine, LevelAssets::new(), rescuer, target)
}

#[test]
fn earlier_waking_up_done_changes_later_actor_before_its_animation_slot() {
    let (mut engine, assets, _rescuer, target) = waking_up_creation_order_engine(true);

    engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

    let target_entity = engine
        .world
        .entities
        .get(target)
        .expect("later wake target remains installed");
    assert!(
        !target_entity
            .human_data()
            .expect("target is human")
            .unconscious
    );
    assert_eq!(
        target_entity.element_data().sprite.last_action,
        crate::order::OrderType::StandingUp,
        "the later target must execute the recovery order synchronously installed by WAKING_UP DONE"
    );
}

#[test]
fn later_waking_up_done_defers_already_visited_actor_recovery_animation() {
    let (mut engine, assets, _rescuer, target) = waking_up_creation_order_engine(false);
    let sim = crate::sim_rng::test_context();

    engine.tick_actor_animation_action_change_slots(&sim, &assets);
    let target_entity = engine
        .world
        .entities
        .get(target)
        .expect("earlier wake target remains installed");
    assert!(
        !target_entity
            .human_data()
            .expect("target is human")
            .unconscious
    );
    assert_eq!(
        target_entity.element_data().sprite.last_action,
        crate::order::OrderType::BeingUnconscious,
        "later WAKING_UP DONE cannot retroactively animate an already visited target"
    );

    engine.tick_actor_animation_action_change_slots(&sim, &assets);
    assert_eq!(
        engine
            .world
            .entities
            .get(target)
            .expect("wake target remains installed next pass")
            .element_data()
            .sprite
            .last_action,
        crate::order::OrderType::StandingUp,
        "the recovery order must animate at the target's next creation slot"
    );
}

#[test]
#[should_panic(expected = "WakingUp requires antagonist at legacy slot")]
fn actor_animation_missing_required_antagonist_fails_with_slot_context() {
    let (mut engine, assets, rescuer, _target) = waking_up_creation_order_engine(true);
    let (sequence, element, _) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(rescuer)
        .expect("rescuer has its WakingUp order");
    engine
        .orders
        .sequence_manager
        .get_element_mut(sequence, element)
        .expect("rescuer WakingUp element remains installed")
        .orders
        .front_mut()
        .expect("rescuer WakingUp order remains installed")
        .antagonist = None;

    engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);
}

#[test]
fn npc_searching_animation_allows_missing_antagonist() {
    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_scripted_soldier(""));
    bind_test_actor_animations(&mut engine, actor, &[crate::order::OrderType::Searching]);
    let order_id = engine.orders.allocate_order_id();
    install_test_order_queue(
        &mut engine,
        actor,
        [crate::order::Order::new(
            crate::order::OrderType::Searching,
            0.0,
            0.0,
            order_id,
        )],
    );

    engine.tick_actor_animation_for(&crate::sim_rng::test_context(), &LevelAssets::new(), actor);

    assert_eq!(
        engine
            .world
            .entities
            .get(actor)
            .expect("searching NPC remains installed")
            .element_data()
            .sprite
            .last_action,
        crate::order::OrderType::Searching,
        "NPC Searching must preserve the Original optional antagonist path"
    );
}

#[test]
#[should_panic(expected = "required Searching antagonist")]
fn npc_searching_animation_rejects_present_stale_antagonist() {
    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_scripted_soldier(""));
    let stale = engine.add_entity(make_scripted_soldier(""));
    engine.remove_entity(stale);
    bind_test_actor_animations(&mut engine, actor, &[crate::order::OrderType::Searching]);
    let order_id = engine.orders.allocate_order_id();
    install_test_order_queue(
        &mut engine,
        actor,
        [
            crate::order::Order::new(crate::order::OrderType::Searching, 0.0, 0.0, order_id)
                .with_antagonist(stale),
        ],
    );

    engine.tick_actor_animation_for(&crate::sim_rng::test_context(), &LevelAssets::new(), actor);
}

// ───────── Owner-local SetState notifications ─────────

fn make_scripted_civilian(script_class: &str) -> Entity {
    Entity::Civilian(ActorCivilian {
        element: ElementData {
            kind: ElementKind::ActorCivilian,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        },
        actor: ActorData {
            script_class: script_class.into(),
            ..ActorData::default()
        },
        human: HumanData::default(),
        npc: NpcData {
            life_points: 50,
            ai_brain: AiBrain::Friendly(Box::default()),
            ..NpcData::default()
        },
        civilian: CivilianData::default(),
    })
}

fn q_set_custom(actor: u16, index: u16, value: u16) -> [Quad; 4] {
    [
        q_native_param(actor),
        q_native_param(index),
        q_native_param(value),
        q_native_call(crate::natives::NativeFn::SetCustomNPCValue as u32),
    ]
}

/// A real SCB FilterAIEvent body which records source/code, observes
/// GetAIState/GetAIAlertStatus through natives, preserves FIFO codes in custom
/// slots 4+, optionally mutates its own AI state, and can mutate a later owner.
fn state_change_filter_class(
    class_name: &str,
    mutate_state: bool,
    mutate_other: Option<i32>,
) -> ClassEntry {
    let mut quads = vec![
        q_begin_function(0, 6),
        q_aff1_get_param(TMP0, 0),
        q_aff1_get_param(TMP1, 4),
        q_native_call(crate::natives::NativeFn::ThisActor as u32),
        q_aff1_native_get_return(TMP2),
        q_aff0_iconstant(TMP3, 0),
    ];
    quads.extend(q_set_custom(TMP2, TMP3, TMP0));
    quads.push(q_aff0_iconstant(TMP3, 1));
    quads.extend(q_set_custom(TMP2, TMP3, TMP1));

    quads.extend([
        q_native_param(TMP2),
        q_native_call(crate::natives::NativeFn::GetAIState as u32),
        q_aff1_native_get_return(TMP4),
        q_aff0_iconstant(TMP3, 2),
    ]);
    quads.extend(q_set_custom(TMP2, TMP3, TMP4));
    if mutate_state {
        // Reserve slot 3 for proof that the StartThink(NO_EVENT) callback
        // received the exact NULL source. Store source+1 only for code -2;
        // the expected marker is therefore 1, not an ambiguous zero.
        quads.extend([q_aff0_iconstant(TMP5, -2), q_ieq(TMP5, TMP1, TMP5)]);
        let skip_no_event_marker = quads.len();
        quads.push(q_if_not_zero_goto(TMP5, 0));
        // Patch this as IfZero below once the target is known.
        quads[skip_no_event_marker].operation = Opcode::IfZeroGoto as u8;
        quads.extend([
            q_aff0_iconstant(TMP5, 1),
            q_iadd(TMP4, TMP0, TMP5),
            q_aff0_iconstant(TMP3, 3),
        ]);
        quads.extend(q_set_custom(TMP2, TMP3, TMP4));
        let after_no_event_marker = quads.len() as u32;
        quads[skip_no_event_marker].operands[4..8]
            .copy_from_slice(&after_no_event_marker.to_le_bytes());
    } else {
        quads.extend([
            q_native_param(TMP2),
            q_native_call(crate::natives::NativeFn::GetAIAlertStatus as u32),
            q_aff1_native_get_return(TMP4),
            q_aff0_iconstant(TMP3, 3),
        ]);
        quads.extend(q_set_custom(TMP2, TMP3, TMP4));
    }

    // Slot 9 is a caller-seeded next-index cursor (initially 4). Record each
    // event code at that index, then increment the cursor.
    quads.extend([
        q_aff0_iconstant(TMP3, 9),
        q_native_param(TMP2),
        q_native_param(TMP3),
        q_native_call(crate::natives::NativeFn::GetCustomNPCValue as u32),
        q_aff1_native_get_return(TMP4),
    ]);
    quads.extend(q_set_custom(TMP2, TMP4, TMP1));
    quads.extend([q_aff0_iconstant(TMP5, 1), q_iadd(TMP4, TMP4, TMP5)]);
    quads.extend(q_set_custom(TMP2, TMP3, TMP4));

    // Record a cross-owner marker before any optional mutation. The observer
    // copies slot 8 to slot 7, making creation-order visibility explicit.
    quads.extend([
        q_aff0_iconstant(TMP3, 8),
        q_native_param(TMP2),
        q_native_param(TMP3),
        q_native_call(crate::natives::NativeFn::GetCustomNPCValue as u32),
        q_aff1_native_get_return(TMP4),
        q_aff0_iconstant(TMP3, 7),
    ]);
    quads.extend(q_set_custom(TMP2, TMP3, TMP4));

    if mutate_state {
        // Recurse only from the outer callback. Slot 8 is a script-visible
        // guard so the nested Fleeing and ScriptDriven callbacks still run
        // and record their event codes without recursing forever.
        quads.extend([
            q_aff0_iconstant(TMP3, 8),
            q_native_param(TMP2),
            q_native_param(TMP3),
            q_native_call(crate::natives::NativeFn::GetCustomNPCValue as u32),
            q_aff1_native_get_return(TMP4),
        ]);
        let skip_mutation = quads.len();
        quads.push(q_if_not_zero_goto(TMP4, 0));
        quads.push(q_aff0_iconstant(TMP4, 1));
        quads.extend(q_set_custom(TMP2, TMP3, TMP4));
        quads.extend([
            // Public AISTATE_FLEEING is 5 (the internal enum is 6).
            q_aff0_iconstant(TMP3, 5),
            q_native_param(TMP2),
            q_native_param(TMP3),
            q_native_call(crate::natives::NativeFn::SetAIState as u32),
            // This marker is deliberately adjacent to the native. Every
            // nested callback copies the guard from slot 8 into slot 7. A
            // true per-native barrier leaves slot 7 at the pre-marker value
            // 1; an end-of-VM drain would let callbacks observe 77 instead.
            q_aff0_iconstant(TMP4, 77),
            q_aff0_iconstant(TMP3, 8),
        ]);
        quads.extend(q_set_custom(TMP2, TMP3, TMP4));
        let after_mutation = quads.len() as u32;
        quads[skip_mutation] = q_if_not_zero_goto(TMP4, after_mutation);
    }
    if let Some(other) = mutate_other {
        quads.extend([
            q_aff0_iconstant(TMP3, other),
            q_aff0_iconstant(TMP4, 8),
            q_aff0_iconstant(TMP5, 77),
        ]);
        quads.extend(q_set_custom(TMP3, TMP4, TMP5));
    }

    // State-change notification return values are ignored; zero is the
    // strongest regression sentinel because StartThink would reject it.
    quads.extend([
        q_aff0_iconstant(TMP0, 0),
        q_return_val(TMP0),
        q_end_function(),
    ]);
    ClassEntry {
        source_file: "state_change_test.scs".into(),
        class_name: class_name.into(),
        size_of_member_variables: 0,
        member_variables: vec![],
        functions: vec![Function {
            name: "FilterAIEvent".into(),
            address: 0,
            num_parameters: 3,
            size_of_return_value: 4,
            size_of_parameters: 12,
            size_of_volatile: 0,
            size_of_temporary: 24,
        }],
        quads,
    }
}

/// Fleeing probe whose NO_EVENT callback changes the actor to ScriptDriven.
/// Panic must classify `is_new_panic` only after this callback returns.
fn post_filter_panic_class(class_name: &str) -> ClassEntry {
    let mut quads = vec![
        q_begin_function(0, 5),
        q_aff1_get_param(TMP0, 4),
        q_native_call(crate::natives::NativeFn::ThisActor as u32),
        q_aff1_native_get_return(TMP1),
        q_aff0_iconstant(TMP2, 9),
        q_native_param(TMP1),
        q_native_param(TMP2),
        q_native_call(crate::natives::NativeFn::GetCustomNPCValue as u32),
        q_aff1_native_get_return(TMP3),
    ];
    quads.extend(q_set_custom(TMP1, TMP3, TMP0));
    quads.extend([q_aff0_iconstant(TMP4, 1), q_iadd(TMP3, TMP3, TMP4)]);
    quads.extend(q_set_custom(TMP1, TMP2, TMP3));
    quads.extend([q_aff0_iconstant(TMP2, -2), q_ieq(TMP2, TMP0, TMP2)]);
    let skip_mutation = quads.len();
    quads.push(q_if_not_zero_goto(TMP2, 0));
    quads[skip_mutation].operation = Opcode::IfZeroGoto as u8;
    quads.extend([
        q_aff0_iconstant(TMP2, 7),
        q_native_param(TMP1),
        q_native_param(TMP2),
        q_native_call(crate::natives::NativeFn::SetAIState as u32),
        q_native_param(TMP1),
        q_native_call(crate::natives::NativeFn::GetAIState as u32),
        q_aff1_native_get_return(TMP3),
        q_aff0_iconstant(TMP2, 0),
    ]);
    quads.extend(q_set_custom(TMP1, TMP2, TMP3));
    let after_mutation = quads.len() as u32;
    quads[skip_mutation].operands[4..8].copy_from_slice(&after_mutation.to_le_bytes());
    quads.extend([
        q_aff0_iconstant(TMP0, 1),
        q_return_val(TMP0),
        q_end_function(),
    ]);

    let run_address = quads.len() as i32;
    quads.extend([
        q_begin_function(0, 3),
        q_native_call(crate::natives::NativeFn::ThisActor as u32),
        q_aff1_native_get_return(TMP0),
        q_aff0_iconstant(TMP1, 5),
        q_native_param(TMP0),
        q_native_param(TMP1),
        q_native_call(crate::natives::NativeFn::SetAIState as u32),
        q_aff0_iconstant(TMP1, 8),
        q_aff0_iconstant(TMP2, 77),
    ]);
    quads.extend(q_set_custom(TMP0, TMP1, TMP2));
    quads.extend([q_return(), q_end_function()]);

    ClassEntry {
        source_file: "post_filter_panic_test.scs".into(),
        class_name: class_name.into(),
        size_of_member_variables: 0,
        member_variables: vec![],
        functions: vec![
            Function {
                name: "FilterAIEvent".into(),
                address: 0,
                num_parameters: 3,
                size_of_return_value: 4,
                size_of_parameters: 12,
                size_of_volatile: 0,
                size_of_temporary: 20,
            },
            Function {
                name: "Run".into(),
                address: run_address,
                num_parameters: 0,
                size_of_return_value: 0,
                size_of_parameters: 0,
                size_of_volatile: 0,
                size_of_temporary: 12,
            },
        ],
        quads,
    }
}

fn state_change_scb(classes: Vec<ClassEntry>) -> ScbFile {
    let mut all = vec![ClassEntry {
        source_file: "state_change_test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: vec![],
        functions: vec![],
        quads: vec![],
    }];
    all.extend(classes);
    ScbFile {
        version: crate::scb::SCB_VERSION,
        classes: all,
    }
}

/// Real SCB class with a `Run` method that invokes SetAIState and immediately
/// snapshots the callback marker, AI state, and live sequence action. The
/// instruction adjacency is intentional: an end-of-VM drain leaves these
/// three observations stale.
fn ai_state_native_probe_class(
    class_name: &str,
    public_state: i32,
    filter_return: i32,
) -> ClassEntry {
    let mut quads = vec![
        q_begin_function(0, 3),
        q_aff1_get_param(TMP0, 4),
        q_native_call(crate::natives::NativeFn::ThisActor as u32),
        q_aff1_native_get_return(TMP1),
        q_aff0_iconstant(TMP2, 0),
    ];
    quads.extend(q_set_custom(TMP1, TMP2, TMP0));
    // Preserve callback FIFO independently from slot 0, which intentionally
    // tracks only the last callback. Bind fixtures seed slot 9 to 4.
    quads.extend([
        q_aff0_iconstant(TMP2, 9),
        q_native_param(TMP1),
        q_native_param(TMP2),
        q_native_call(crate::natives::NativeFn::GetCustomNPCValue as u32),
        q_aff1_native_get_return(TMP2),
    ]);
    quads.extend(q_set_custom(TMP1, TMP2, TMP0));
    quads.extend([
        q_aff0_iconstant(TMP0, 1),
        q_iadd(TMP2, TMP2, TMP0),
        q_aff0_iconstant(TMP0, 9),
    ]);
    quads.extend(q_set_custom(TMP1, TMP0, TMP2));
    quads.extend([
        q_aff0_iconstant(TMP0, filter_return),
        q_return_val(TMP0),
        q_end_function(),
    ]);
    let run_address = quads.len() as i32;
    quads.extend([
        q_begin_function(0, 4),
        q_native_call(crate::natives::NativeFn::ThisActor as u32),
        q_aff1_native_get_return(TMP0),
        q_aff0_iconstant(TMP1, public_state),
        q_native_param(TMP0),
        q_native_param(TMP1),
        q_native_call(crate::natives::NativeFn::SetAIState as u32),
        // Adjacent observation 1: last state/NO_EVENT callback code.
        q_aff0_iconstant(TMP1, 0),
        q_native_param(TMP0),
        q_native_param(TMP1),
        q_native_call(crate::natives::NativeFn::GetCustomNPCValue as u32),
        q_aff1_native_get_return(TMP2),
        q_aff0_iconstant(TMP1, 1),
    ]);
    quads.extend(q_set_custom(TMP0, TMP1, TMP2));
    quads.extend([
        // Adjacent observation 2: committed typed state.
        q_native_param(TMP0),
        q_native_call(crate::natives::NativeFn::GetAIState as u32),
        q_aff1_native_get_return(TMP2),
        q_aff0_iconstant(TMP1, 2),
    ]);
    quads.extend(q_set_custom(TMP0, TMP1, TMP2));
    quads.extend([
        // Adjacent observation 3: live SequenceManager order.
        q_native_param(TMP0),
        q_native_call(crate::natives::NativeFn::GetCurrentAction as u32),
        q_aff1_native_get_return(TMP2),
        q_aff0_iconstant(TMP1, 3),
    ]);
    quads.extend(q_set_custom(TMP0, TMP1, TMP2));
    quads.extend([q_return(), q_end_function()]);

    ClassEntry {
        source_file: "ai_state_native_probe.scs".into(),
        class_name: class_name.into(),
        size_of_member_variables: 0,
        member_variables: vec![],
        functions: vec![
            Function {
                name: "FilterAIEvent".into(),
                address: 0,
                num_parameters: 3,
                size_of_return_value: 4,
                size_of_parameters: 12,
                size_of_volatile: 0,
                size_of_temporary: 12,
            },
            Function {
                name: "Run".into(),
                address: run_address,
                num_parameters: 0,
                size_of_return_value: 0,
                size_of_parameters: 0,
                size_of_volatile: 0,
                size_of_temporary: 16,
            },
        ],
        quads,
    }
}

fn install_state_change_script(engine: &mut EngineInner, scb: ScbFile) -> LevelAssets {
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    engine.scripts.mission = Some(MissionScript::from_scb(scb).expect("state-change SCB builds"));
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    assets
}

fn bind_state_change_actor(engine: &mut EngineInner, actor: EntityId, class_name: &str) -> i32 {
    let handle = crate::natives::ScriptHandleCodec::actor_handle(actor);
    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut engine.world.entities,
        &mut engine.ai.global,
        &mut engine.world.fast_grid,
    );
    assert!(
        engine
            .scripts
            .mission
            .as_mut()
            .expect("state-change script installed")
            .bind_actor(
                handle,
                class_name,
                &mut engine.script_domains,
                &capabilities,
            )
    );
    engine
        .world
        .entities
        .get_mut(actor)
        .expect("bound state-change actor exists")
        .npc_data_mut()
        .expect("bound state-change actor is an NPC")
        .custom_values[9] = 4;
    handle
}

fn npc_custom_values(engine: &EngineInner, actor: EntityId) -> [i32; 10] {
    engine
        .world
        .entities
        .get(actor)
        .expect("state-change actor exists")
        .npc_data()
        .expect("state-change actor is an NPC")
        .custom_values
}

fn run_ai_state_native_probe(engine: &mut EngineInner, assets: &LevelAssets, actor: EntityId) {
    let handle = crate::natives::ScriptHandleCodec::actor_handle(actor);
    engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            assets,
            crate::engine::ScriptVmKey::Actor(handle),
            "Run",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("real SCB SetAIState probe completes");
}

fn setup_ai_state_native_probe(
    class_name: &str,
    public_state: i32,
) -> (EngineInner, LevelAssets, EntityId) {
    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_scripted_soldier(class_name));
    let mut assets = install_state_change_script(
        &mut engine,
        state_change_scb(vec![ai_state_native_probe_class(
            class_name,
            public_state,
            1,
        )]),
    );
    {
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles.hth_weapons.push(Default::default());
        profiles.characters.push(crate::profiles::CharacterProfile {
            hth_weapon_id: 1,
            ..Default::default()
        });
        profiles.soldiers.push(crate::profiles::SoldierProfile {
            profile_name: format!("{class_name}-profile"),
            exclamation_id: 501,
            hth_weapon_id: 1,
            ..Default::default()
        });
    }
    bind_state_change_actor(&mut engine, actor, class_name);
    engine
        .world
        .entities
        .get_mut(actor)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;

    let seek_sector = crate::position_interface::SectorHandle::new(1).unwrap();
    let element = engine
        .world
        .entities
        .get_mut(actor)
        .unwrap()
        .element_data_mut();
    element.set_position(WorldPoint3D::new(100.0, 100.0, 0.0));
    element.set_layer(0);
    element.set_sector(Some(seek_sector));
    element
        .sprite
        .position_iface
        .set_move_box(crate::coordinates::MoveBox::from_corners(
            crate::coordinates::MapVec::new(-2.0, -2.0),
            crate::coordinates::MapVec::new(2.0, 2.0),
        ));
    engine.ai.global.seek_points.push(crate::ai::SeekPoint {
        position: crate::ai::Position {
            x: 200.0,
            y: 100.0,
            sector: Some(seek_sector),
            level: 0,
        },
        frame_when_full_interest: 0,
        directions: vec![0],
        last_calculated_interest: 100,
        locked: false,
        id: 0,
    });
    (engine, assets, actor)
}

fn install_unrelated_multi_exit_building_actor(engine: &mut EngineInner) {
    use crate::element::ActiveDoorPass;
    use crate::fast_find_grid::GridSector;
    use crate::gate::{Door, DoorIndex, DoorType};
    use crate::sector::{SectorNumber, SectorType};
    use std::collections::VecDeque;

    let door_actor = engine.add_entity(make_pc(true));
    let Entity::Pc(pc) = engine
        .world
        .entities
        .get_mut(door_actor)
        .expect("unrelated door-passing actor exists")
    else {
        panic!("unrelated door-passing actor changed kind")
    };
    pc.element.active = true;
    pc.pc.life_points = 100;
    pc.actor.active_door_pass = Some(ActiveDoorPass {
        door_index: DoorIndex(0),
        direct: true,
        steps: VecDeque::new(),
        triggers_fired: 0,
        current_action: crate::order::OrderType::default(),
        current_reverse: false,
        saved_action_state: None,
    });

    let building_sector = SectorNumber::new(8);
    engine.script_domains.interactables.doors = vec![
        Door {
            door_type: DoorType::Building,
            sector_out: SectorNumber::new(7),
            sector_in: building_sector,
            point_out: crate::coordinates::MapPoint::new(0.0, 0.0),
            point_in: crate::coordinates::MapPoint::new(10.0, 0.0),
            ..Door::default()
        },
        Door {
            door_type: DoorType::Building,
            sector_out: SectorNumber::new(9),
            sector_in: building_sector,
            point_out: crate::coordinates::MapPoint::new(100.0, 0.0),
            point_in: crate::coordinates::MapPoint::new(90.0, 0.0),
            ..Door::default()
        },
    ];
    let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid.level);
    level.sector_number_map.insert(building_sector, 0);
    level.sectors.push(GridSector {
        points: Vec::new(),
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type: SectorType::BUILDING,
        layer: 0,
        sector_number: building_sector,
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

#[test]
fn script_native_state_effects_stabilize_before_adjacent_instruction() {
    let mut engine = EngineInner::new();
    let script_driven = engine.add_entity(make_scripted_soldier("ScriptDrivenProbe"));
    let seeking = engine.add_entity(make_scripted_soldier("SeekingProbe"));
    let seeking_filter_zero = engine.add_entity(make_scripted_soldier("SeekingFilterZero"));
    let seeking_at_point = engine.add_entity(make_scripted_soldier("SeekingAtPoint"));
    let default = engine.add_entity(make_scripted_soldier("DefaultProbe"));
    let mut assets = install_state_change_script(
        &mut engine,
        state_change_scb(vec![
            ai_state_native_probe_class("ScriptDrivenProbe", 7, 1),
            ai_state_native_probe_class("SeekingProbe", 3, 1),
            ai_state_native_probe_class("SeekingFilterZero", 3, 0),
            ai_state_native_probe_class("SeekingAtPoint", 3, 1),
            ai_state_native_probe_class("DefaultProbe", 1, 1),
        ]),
    );
    {
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles.hth_weapons.push(Default::default());
        profiles.soldiers.push(crate::profiles::SoldierProfile {
            profile_name: "native-probe".into(),
            hth_weapon_id: 1,
            ..Default::default()
        });
    }
    for (actor, class_name) in [
        (script_driven, "ScriptDrivenProbe"),
        (seeking, "SeekingProbe"),
        (seeking_filter_zero, "SeekingFilterZero"),
        (seeking_at_point, "SeekingAtPoint"),
        (default, "DefaultProbe"),
    ] {
        bind_state_change_actor(&mut engine, actor, class_name);
        engine
            .world
            .entities
            .get_mut(actor)
            .unwrap()
            .enemy_ai_mut()
            .unwrap()
            .hth_weapon_id = 1;
    }
    let seek_sector = crate::position_interface::SectorHandle::new(1).unwrap();
    for (actor, x) in [
        (seeking, 100.0),
        (seeking_filter_zero, 110.0),
        (seeking_at_point, 198.0),
    ] {
        let element = engine
            .world
            .entities
            .get_mut(actor)
            .unwrap()
            .element_data_mut();
        element.set_position(WorldPoint3D::new(x, 100.0, 0.0));
        element.set_layer(0);
        element.set_sector(Some(seek_sector));
        element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                crate::coordinates::MapVec::new(-2.0, -2.0),
                crate::coordinates::MapVec::new(2.0, 2.0),
            ));
        if actor == seeking_at_point {
            engine
                .world
                .entities
                .get_mut(actor)
                .unwrap()
                .actor_data_mut()
                .unwrap()
                .old_action = crate::order::OrderType::WaitingUpright;
        }
    }
    engine.ai.global.seek_points.push(crate::ai::SeekPoint {
        position: crate::ai::Position {
            x: 200.0,
            y: 100.0,
            sector: Some(seek_sector),
            level: 0,
        },
        frame_when_full_interest: 0,
        directions: vec![0],
        last_calculated_interest: 100,
        locked: false,
        id: 0,
    });
    engine
        .world
        .entities
        .get_mut(default)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .set_state(
            crate::ai::AiState::Seeking,
            crate::ai::Substate::SeekingJustWatching,
        );
    engine.drain_ai_state_change_notifications_for(
        &crate::sim_rng::test_context(),
        &assets,
        default,
    );
    engine
        .world
        .entities
        .get_mut(default)
        .unwrap()
        .npc_data_mut()
        .unwrap()
        .custom_values = [0; 10];

    run_ai_state_native_probe(&mut engine, &assets, script_driven);
    assert_eq!(
        &npc_custom_values(&engine, script_driven)[0..3],
        &[101, 101, 1],
        "ScriptDriven callback and real typed assignment precede the next instruction"
    );

    run_ai_state_native_probe(&mut engine, &assets, seeking);
    let seeking_values = npc_custom_values(&engine, seeking);
    assert_eq!(&seeking_values[0..3], &[103, 103, 3]);
    assert_eq!(
        seeking_values[3],
        crate::order::OrderType::TransitionWaitingUprightRunningUpright as i32,
        "SeekArea translates its GoTo transition before VM resumption"
    );

    engine.ai.global.seek_points[0].locked = false;
    run_ai_state_native_probe(&mut engine, &assets, seeking_filter_zero);
    let zero_values = npc_custom_values(&engine, seeking_filter_zero);
    assert_eq!(&zero_values[0..3], &[103, 103, 3]);
    assert_eq!(
        zero_values[3],
        crate::order::OrderType::TransitionWaitingUprightRunningUpright as i32,
        "FilterAIEvent returning zero still allows SetAIState's SeekArea effect"
    );

    engine.ai.global.seek_points[0].locked = false;
    run_ai_state_native_probe(&mut engine, &assets, seeking_at_point);
    let at_point_values = npc_custom_values(&engine, seeking_at_point);
    assert_eq!(
        &at_point_values[4..8],
        &[-2, 103, 3, 103],
        "EndThink dispatches AI_EVENT_REACHPOINT before its recursive SeekNextPoint state callback"
    );
    assert_eq!(
        at_point_values[0], 103,
        "the complete recursive fixed point settles before the native-adjacent instruction"
    );
    let at_point_ai = engine
        .world
        .entities
        .get(seeking_at_point)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert!(!at_point_ai.already_on_point);
    assert_eq!(at_point_ai.think_recursion_depth, 0);

    run_ai_state_native_probe(&mut engine, &assets, default);
    assert_eq!(
        &npc_custom_values(&engine, default)[0..3],
        &[4, 4, 1],
        "Think(EVENT_RETURN_TO_DUTY) is the synchronous callback and commits Default"
    );
}

#[test]
fn pre_existing_same_owner_moves_are_stopped_without_being_dispatched_as_causal_move() {
    let (mut engine, assets, actor) = setup_ai_state_native_probe("MoveSentinelProbe", 3);
    let pending =
        crate::order::AiOrderIntent::new(crate::order::OrderType::WalkingUpright, 333.0, 444.0);
    engine.orders.pending_move_requests.push((actor, pending));

    let mut deferred = crate::sequence::SequenceElement::new_movement(
        1,
        crate::element::Command::Move,
        Some(actor),
        crate::order::OrderType::WalkingWithSword,
    );
    let deferred_destination = crate::coordinates::MapPoint::new(555.0, 666.0);
    let crate::sequence::SequenceElementData::Movement { destination, .. } = &mut deferred.data
    else {
        unreachable!("new_movement must construct movement data")
    };
    *destination = deferred_destination;
    let deferred_sequence = engine.orders.sequence_manager.launch_element(deferred);

    run_ai_state_native_probe(&mut engine, &assets, actor);

    assert_eq!(
        npc_custom_values(&engine, actor)[3],
        crate::order::OrderType::TransitionWaitingUprightRunningUpright as i32,
        "the causal SeekArea Move translates before VM resumption"
    );
    assert!(
        engine.orders.pending_move_requests.is_empty(),
        "StopAll/Halt cancels the older pre-sequence Move intent before SeekArea queues its causal Move"
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.owner == Some(actor)
                    && matches!(
                        &element.data,
                        crate::sequence::SequenceElementData::Movement { destination, .. }
                            if *destination == crate::coordinates::MapPoint::new(333.0, 444.0)
                    )
            }),
        "the stale pending 333/444 intent must be dropped at Halt, never translated before the causal Move"
    );

    let deferred_after = engine
        .orders
        .sequence_manager
        .get_element(deferred_sequence, 0)
        .expect("stopped deferred Move remains inspectable until sequence cleanup");
    assert_eq!(deferred_after.command, crate::element::Command::Move);
    assert_eq!(
        deferred_after.state,
        crate::sequence::SequenceState::Interrupted,
        "Original Stop(PREFERENCE) intentionally interrupts the older deferred Move"
    );
    let crate::sequence::SequenceElementData::Movement {
        destination,
        action,
        ..
    } = &deferred_after.data
    else {
        panic!("pre-existing deferred Move changed data kind")
    };
    assert_eq!(*destination, deferred_destination);
    assert_eq!(*action, crate::order::OrderType::WalkingWithSword);
    assert!(
        !engine
            .orders
            .sequence_manager
            .hourglass()
            .iter()
            .any(|action| {
                matches!(
                    action,
                    crate::sequence::SequenceAction::InstructOwner {
                        owner,
                        sequence_id,
                        element_index: 0,
                    } if *owner == actor && *sequence_id == deferred_sequence
                )
            }),
        "the stopped old Move must not be mistaken for the exact causal sequence ID"
    );
}

#[test]
fn set_ai_state_seeking_and_fleeing_do_not_draw_unrelated_building_exit_gate_rng() {
    use crate::sim_rng::{RngSite, with_draw_trace};

    let sim = crate::sim_rng::test_context();
    let (mut seeking_engine, seeking_assets, seeking) =
        setup_ai_state_native_probe("SeekingRngProbe", 3);
    install_unrelated_multi_exit_building_actor(&mut seeking_engine);
    {
        let entity = seeking_engine.world.entities.get_mut(seeking).unwrap();
        entity
            .element_data_mut()
            .set_position(WorldPoint3D::new(198.0, 100.0, 0.0));
        entity.actor_data_mut().unwrap().old_action = crate::order::OrderType::WaitingUpright;
    }
    let (_, control_trace) = with_draw_trace(|| {
        drop(seeking_engine.build_sim_scratch(&sim, &seeking_assets));
    });
    assert!(
        control_trace.contains(&RngSite::BuildingExitGate),
        "the unrelated multi-exit fixture must exercise BuildingExitGate under global forecasting"
    );
    let (_, seeking_trace) = with_draw_trace(|| {
        run_ai_state_native_probe(&mut seeking_engine, &seeking_assets, seeking);
    });
    assert!(
        !seeking_trace.contains(&RngSite::BuildingExitGate),
        "Seeking, including at-point EndThink recursion, must remain owner-local and forecast-free"
    );
    assert_eq!(
        &npc_custom_values(&seeking_engine, seeking)[4..8],
        &[-2, 103, 3, 103],
        "the RNG-free recursive drain must still invoke FilterAIEvent for EventReachPoint"
    );
    let seeking_ai = seeking_engine
        .world
        .entities
        .get(seeking)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert!(!seeking_ai.already_on_point);
    assert_eq!(seeking_ai.think_recursion_depth, 0);

    let (mut fleeing_engine, fleeing_assets, fleeing) =
        setup_ai_state_native_probe("FleeingRngProbe", 5);
    install_unrelated_multi_exit_building_actor(&mut fleeing_engine);
    let (_, fleeing_trace) = with_draw_trace(|| {
        run_ai_state_native_probe(&mut fleeing_engine, &fleeing_assets, fleeing);
    });
    assert!(
        !fleeing_trace.contains(&RngSite::BuildingExitGate),
        "Fleeing/Panic must not forecast an unrelated building actor"
    );
}

#[test]
#[should_panic(expected = "accepted SetAIState soldier 268435456 requires Enemy AI")]
fn malformed_soldier_with_friendly_brain_fails_set_ai_state_contextually() {
    let (mut engine, assets, actor) = setup_ai_state_native_probe("MalformedBrainProbe", 3);
    let Entity::Soldier(soldier) = engine.world.entities.get_mut(actor).unwrap() else {
        unreachable!("probe actor changed kind")
    };
    soldier.npc.ai_brain = AiBrain::Friendly(Box::default());

    run_ai_state_native_probe(&mut engine, &assets, actor);
}

#[test]
fn fleeing_panic_classification_occurs_after_no_event_callback_mutation() {
    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_scripted_soldier("PostFilterPanicProbe"));
    let mut assets = install_state_change_script(
        &mut engine,
        state_change_scb(vec![post_filter_panic_class("PostFilterPanicProbe")]),
    );
    {
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles.hth_weapons.push(Default::default());
        profiles.soldiers.push(crate::profiles::SoldierProfile {
            profile_name: "post-filter-panic".into(),
            exclamation_id: 501,
            hth_weapon_id: 1,
            ..Default::default()
        });
    }
    bind_state_change_actor(&mut engine, actor, "PostFilterPanicProbe");
    engine.world.fast_grid.size_map(64, 64);
    engine.world.fast_grid.allocate_layers(1);
    let sector = crate::position_interface::SectorHandle::new(1).unwrap();
    {
        let element = engine
            .world
            .entities
            .get_mut(actor)
            .unwrap()
            .element_data_mut();
        // Keep every randomized panic segment inside a real, obstacle-free
        // grid. Otherwise the fixture exercises the 111-call failed-path
        // recursion fallback and ReturnToDuty instead of isolating the
        // post-filter SetState commit.
        element.set_position(WorldPoint3D::new(1000.0, 1000.0, 0.0));
        element.set_layer(0);
        element.set_sector(Some(sector));
        element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                crate::coordinates::MapVec::new(-2.0, -2.0),
                crate::coordinates::MapVec::new(2.0, 2.0),
            ));
    }
    {
        let ai = engine
            .world
            .entities
            .get_mut(actor)
            .unwrap()
            .enemy_ai_mut()
            .unwrap();
        ai.hth_weapon_id = 1;
        ai.base.current_state = crate::ai::AiState::Fleeing;
        ai.base.current_substate = crate::ai::Substate::FleeingPanic;
        ai.base.current_remark = crate::ai::Remark::TheSoundOfSilence;
    }

    run_ai_state_native_probe(&mut engine, &assets, actor);

    let values = npc_custom_values(&engine, actor);
    assert_eq!(
        &values[4..7],
        &[-2, 101, 106],
        "NO_EVENT callback mutates to ScriptDriven before Panic classifies and emits its state callback"
    );
    assert_eq!(
        values[0], 1,
        "the NO_EVENT callback observes nested ScriptDriven committed before it returns"
    );
    assert_eq!(values[8], 77, "the adjacent instruction runs last");
    let ai = engine
        .world
        .entities
        .get(actor)
        .unwrap()
        .enemy_ai()
        .unwrap();
    assert_eq!(ai.base.current_state, crate::ai::AiState::Fleeing);
    assert_eq!(ai.base.current_substate, crate::ai::Substate::FleeingPanic);
    assert_eq!(
        ai.base.current_remark,
        crate::ai::Remark::Panic,
        "post-callback ScriptDriven state makes this a new panic with synchronous speech"
    );
    assert_eq!(ai.base.think_recursion_depth, 0);
}

#[test]
fn set_ai_state_ignores_start_think_freeze_script_lock_and_ai_lock_results() {
    for (class_name, refusal, configure) in [
        ("StaticFreezeProbe", 1, 0_u8),
        ("ScriptLockProbe", 2, 1_u8),
        ("AiLockProbe", 3, 2_u8),
    ] {
        let (mut engine, assets, actor) = setup_ai_state_native_probe(class_name, 3);
        match configure {
            0 => engine.ai.global.freeze = true,
            1 => {
                let ai = engine
                    .world
                    .entities
                    .get_mut(actor)
                    .unwrap()
                    .ai_controller_mut()
                    .unwrap();
                ai.script_locked = true;
                ai.remember_events = true;
            }
            2 => {
                engine
                    .world
                    .entities
                    .get_mut(actor)
                    .unwrap()
                    .ai_controller_mut()
                    .unwrap()
                    .locks_flag_field = crate::ai::AiLockFlags::FREEZE;
            }
            _ => unreachable!(),
        }

        run_ai_state_native_probe(&mut engine, &assets, actor);

        let values = npc_custom_values(&engine, actor);
        assert_eq!(
            values[4], -2,
            "{class_name} must run FilterAIEvent(NULL, NO_EVENT) before its post-filter gate"
        );
        let ai = engine
            .world
            .entities
            .get(actor)
            .unwrap()
            .ai_controller()
            .unwrap();
        assert_eq!(ai.current_state, crate::ai::AiState::Seeking);
        assert_eq!(ai.think_recursion_depth, 0);
        assert!(ai.ai_log.iter().any(|line| {
            line.line_type == crate::ai::LogLineType::EventRefused && line.info == refusal
        }));
    }
}

#[test]
fn enemy_state_change_callback_is_owner_local_observes_outgoing_and_ignores_zero() {
    let mut engine = EngineInner::new();
    let enemy = engine.add_entity(make_scripted_soldier("StateMutator"));
    let mut assets = install_state_change_script(
        &mut engine,
        state_change_scb(vec![state_change_filter_class("StateMutator", true, None)]),
    );
    {
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles.hth_weapons.push(Default::default());
        profiles.soldiers.push(crate::profiles::SoldierProfile {
            profile_name: "state-mutator".into(),
            exclamation_id: 501,
            hth_weapon_id: 1,
            ..Default::default()
        });
    }
    engine
        .world
        .entities
        .get_mut(enemy)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    bind_state_change_actor(&mut engine, enemy, "StateMutator");

    engine
        .world
        .entities
        .get_mut(enemy)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .set_state(
            crate::ai::AiState::Seeking,
            crate::ai::Substate::SeekingHeardsteps,
        );
    engine.drain_ai_state_change_notifications_for(&crate::sim_rng::test_context(), &assets, enemy);

    let values = npc_custom_values(&engine, enemy);
    assert_eq!(
        &values[4..7],
        &[103, -2, 106],
        "outer callback, synchronous NULL/-2 prelude, then Panic state callback"
    );
    assert_eq!(
        values[7], 1,
        "nested callbacks complete before the adjacent instruction writes its marker"
    );
    assert_eq!(
        values[8], 77,
        "adjacent instruction ran after stabilization"
    );
    assert_eq!(values[3], 1, "NO_EVENT callback source was exactly NULL");
    assert_eq!(values[9], 8, "four callbacks ran recursively");
    let ai = engine
        .world
        .entities
        .get(enemy)
        .unwrap()
        .enemy_ai()
        .unwrap();
    assert_eq!(ai.base.current_state, crate::ai::AiState::Seeking);
    assert_eq!(
        ai.base.current_substate,
        crate::ai::Substate::SeekingHeardsteps
    );
    assert_eq!(ai.base.current_remark, crate::ai::Remark::Panic);
    assert_eq!(
        ai.base.think_recursion_depth, 0,
        "nested StartThink/EndThink brackets balance before the outer callback resumes"
    );
    assert!(ai.base.outbox.actor.begin_panic.is_none());
    assert!(ai.base.outbox.reentrant.owner_work.is_empty());
    let speak = ai
        .base
        .ai_log
        .iter()
        .position(|line| {
            line.line_type == crate::ai::LogLineType::Speak
                && line.info == crate::ai::Remark::Panic as u16
        })
        .expect("Panic Say settles at the nested native boundary");
    assert_eq!(
        ai.base.ai_log[speak].info,
        crate::ai::Remark::Panic as u16,
        "Panic Say settled at its exact merged owner-FIFO boundary"
    );
}

#[test]
fn enemy_state_change_sources_and_same_substate_gate_match_original() {
    let mut engine = EngineInner::new();
    let enemy = engine.add_entity(make_scripted_soldier("StateRecorder"));
    let target = engine.add_entity(make_pc(true));
    let target_raw = target.index();
    let target_handle = crate::natives::ScriptHandleCodec::actor_handle(target);
    let assets = install_state_change_script(
        &mut engine,
        state_change_scb(vec![state_change_filter_class(
            "StateRecorder",
            false,
            None,
        )]),
    );
    bind_state_change_actor(&mut engine, enemy, "StateRecorder");
    let sim = crate::sim_rng::test_context();

    {
        let ai = engine
            .world
            .entities
            .get_mut(enemy)
            .unwrap()
            .enemy_ai_mut()
            .unwrap();
        ai.base.primary_target = target_raw;
        ai.set_state(
            crate::ai::AiState::Attacking,
            crate::ai::Substate::AttackingSwordfight,
        );
    }
    engine.drain_ai_state_change_notifications_for(&sim, &assets, enemy);
    assert_eq!(npc_custom_values(&engine, enemy)[0], target_handle);

    engine
        .world
        .entities
        .get_mut(enemy)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .set_state(
            crate::ai::AiState::Attacking,
            crate::ai::Substate::AttackingSwordfight,
        );
    engine.drain_ai_state_change_notifications_for(&sim, &assets, enemy);
    assert_eq!(
        npc_custom_values(&engine, enemy)[9],
        5,
        "same substate is silent"
    );

    {
        let ai = engine
            .world
            .entities
            .get_mut(enemy)
            .unwrap()
            .enemy_ai_mut()
            .unwrap();
        ai.base.primary_target = 0;
        ai.set_state(
            crate::ai::AiState::Fleeing,
            crate::ai::Substate::FleeingPanic,
        );
    }
    engine.drain_ai_state_change_notifications_for(&sim, &assets, enemy);
    let values = npc_custom_values(&engine, enemy);
    assert_eq!(values[0], 0, "NULL primary target stays NULL");
    assert_eq!(values[1], 106);
    assert_eq!(values[9], 6);
}

#[test]
fn friendly_repeated_state_change_callbacks_see_target_alert_and_outgoing_state() {
    let mut engine = EngineInner::new();
    let friendly = engine.add_entity(make_scripted_civilian("FriendlyRecorder"));
    let target = engine.add_entity(make_pc(true));
    let target_handle = crate::natives::ScriptHandleCodec::actor_handle(target);
    let assets = install_state_change_script(
        &mut engine,
        state_change_scb(vec![state_change_filter_class(
            "FriendlyRecorder",
            false,
            None,
        )]),
    );
    bind_state_change_actor(&mut engine, friendly, "FriendlyRecorder");
    let sim = crate::sim_rng::test_context();

    {
        let ai = engine
            .world
            .entities
            .get_mut(friendly)
            .unwrap()
            .friendly_ai_mut()
            .unwrap();
        ai.base.primary_target = target.index();
        ai.set_state(
            crate::ai::AiState::Fleeing,
            crate::ai::Substate::FleeingPanic,
        );
    }
    engine.drain_ai_state_change_notifications_for(&sim, &assets, friendly);
    let first = npc_custom_values(&engine, friendly);
    assert_eq!(first[0], target_handle);
    assert_eq!(first[1], 106);
    assert_eq!(first[2], crate::ai::AiState::Default.to_script_code());
    assert_eq!(first[3], crate::ai::AlertLevel::Yellow as i32);

    engine
        .world
        .entities
        .get_mut(friendly)
        .unwrap()
        .friendly_ai_mut()
        .unwrap()
        .set_state(
            crate::ai::AiState::Fleeing,
            crate::ai::Substate::FleeingPanic,
        );
    engine.drain_ai_state_change_notifications_for(&sim, &assets, friendly);
    let repeated = npc_custom_values(&engine, friendly);
    assert_eq!(&repeated[4..6], &[106, 106]);
    assert_eq!(repeated[9], 6, "Friendly notifies repeated transitions");
}

#[test]
fn owner_state_change_fifo_preserves_every_transition() {
    let mut engine = EngineInner::new();
    let friendly = engine.add_entity(make_scripted_civilian("FriendlyRecorder"));
    let assets = install_state_change_script(
        &mut engine,
        state_change_scb(vec![state_change_filter_class(
            "FriendlyRecorder",
            false,
            None,
        )]),
    );
    bind_state_change_actor(&mut engine, friendly, "FriendlyRecorder");

    {
        let ai = engine
            .world
            .entities
            .get_mut(friendly)
            .unwrap()
            .friendly_ai_mut()
            .unwrap();
        ai.set_state(
            crate::ai::AiState::Wondering,
            crate::ai::Substate::WonderingWatching,
        );
        ai.set_state(
            crate::ai::AiState::Seeking,
            crate::ai::Substate::SeekingJustWatching,
        );
        ai.set_state(
            crate::ai::AiState::Default,
            crate::ai::Substate::DefaultOnPost,
        );
    }
    engine.drain_ai_state_change_notifications_for(
        &crate::sim_rng::test_context(),
        &assets,
        friendly,
    );
    let values = npc_custom_values(&engine, friendly);
    assert_eq!(&values[4..7], &[102, 103, 101]);
    assert_eq!(values[9], 7);
    let ai = engine
        .world
        .entities
        .get(friendly)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert_eq!(ai.current_state, crate::ai::AiState::Default);
    assert_eq!(ai.current_substate, crate::ai::Substate::DefaultOnPost);
}

fn run_cross_owner_state_change_order(mutator_first: bool) -> i32 {
    let mut engine = EngineInner::new();
    let mutator = engine.add_entity(make_scripted_soldier("CrossMutator"));
    let observer = engine.add_entity(make_scripted_soldier("CrossObserver"));
    let observer_handle = crate::natives::ScriptHandleCodec::actor_handle(observer);
    let assets = install_state_change_script(
        &mut engine,
        state_change_scb(vec![
            state_change_filter_class("CrossMutator", false, Some(observer_handle)),
            state_change_filter_class("CrossObserver", false, None),
        ]),
    );
    bind_state_change_actor(&mut engine, mutator, "CrossMutator");
    bind_state_change_actor(&mut engine, observer, "CrossObserver");
    for actor in [mutator, observer] {
        engine
            .world
            .entities
            .get_mut(actor)
            .unwrap()
            .enemy_ai_mut()
            .unwrap()
            .set_state(
                crate::ai::AiState::Seeking,
                crate::ai::Substate::SeekingJustWatching,
            );
    }
    let sim = crate::sim_rng::test_context();
    let order = if mutator_first {
        [mutator, observer]
    } else {
        [observer, mutator]
    };
    for actor in order {
        engine.drain_ai_state_change_notifications_for(&sim, &assets, actor);
    }
    npc_custom_values(&engine, observer)[7]
}

#[test]
fn owner_local_callbacks_expose_only_prior_creation_slots() {
    assert_eq!(run_cross_owner_state_change_order(true), 77);
    assert_eq!(
        run_cross_owner_state_change_order(false),
        0,
        "a later owner callback cannot retroactively change an earlier callback snapshot"
    );
}

#[test]
fn direct_parade_and_special_strike_drain_boundary_does_not_leak() {
    let mut engine = EngineInner::new();
    let enemy = engine.add_entity(make_scripted_soldier("StateRecorder"));
    let assets = install_state_change_script(
        &mut engine,
        state_change_scb(vec![state_change_filter_class(
            "StateRecorder",
            false,
            None,
        )]),
    );
    bind_state_change_actor(&mut engine, enemy, "StateRecorder");
    let sim = crate::sim_rng::test_context();

    {
        let ai = engine
            .world
            .entities
            .get_mut(enemy)
            .unwrap()
            .enemy_ai_mut()
            .unwrap();
        ai.base.set_ai_state(crate::ai::AiState::Attacking);
        ai.base.current_substate = crate::ai::Substate::AttackingSwordfight;
        ai.set_state(
            crate::ai::AiState::Attacking,
            crate::ai::Substate::AttackingSwordfightParade,
        );
    }
    engine.drain_pending_for_npc(&sim, enemy, &assets);
    let parade = npc_custom_values(&engine, enemy);
    assert_eq!(parade[1], 104);
    assert_eq!(parade[2], crate::ai::AiState::Attacking.to_script_code());
    assert_eq!(parade[9], 5);

    {
        let ai = engine
            .world
            .entities
            .get_mut(enemy)
            .unwrap()
            .enemy_ai_mut()
            .unwrap();
        ai.base.current_substate = crate::ai::Substate::AttackingSwordfight;
        ai.begin_special_strike();
    }
    engine.drain_pending_for_npc(&sim, enemy, &assets);
    let ai = engine
        .world
        .entities
        .get(enemy)
        .unwrap()
        .enemy_ai()
        .unwrap();
    assert!(ai.pending_special_strike);
    assert!(ai.base.outbox.reentrant.owner_work.is_empty());
    assert_eq!(
        npc_custom_values(&engine, enemy)[9],
        5,
        "same-substate special strike must not emit an Enemy callback"
    );
}

#[test]
fn unavailable_state_change_callbacks_are_consumed() {
    fn queue_seeking(engine: &mut EngineInner, actor: EntityId) {
        engine
            .world
            .entities
            .get_mut(actor)
            .unwrap()
            .enemy_ai_mut()
            .unwrap()
            .set_state(
                crate::ai::AiState::Seeking,
                crate::ai::Substate::SeekingJustWatching,
            );
    }
    fn assert_consumed(engine: &EngineInner, actor: EntityId) {
        let ai = engine
            .world
            .entities
            .get(actor)
            .unwrap()
            .ai_controller()
            .unwrap();
        assert!(ai.outbox.reentrant.owner_work.is_empty());
        assert_eq!(ai.current_state, crate::ai::AiState::Seeking);
    }

    let assets = LevelAssets::new();
    let sim = crate::sim_rng::test_context();

    let mut no_mission = EngineInner::new();
    let actor = no_mission.add_entity(make_scripted_soldier("StateRecorder"));
    queue_seeking(&mut no_mission, actor);
    no_mission.drain_ai_state_change_notifications_for(&sim, &assets, actor);
    assert_consumed(&no_mission, actor);

    let mut unbound = EngineInner::new();
    let actor = unbound.add_entity(make_scripted_soldier("StateRecorder"));
    let assets = install_state_change_script(
        &mut unbound,
        state_change_scb(vec![state_change_filter_class(
            "StateRecorder",
            false,
            None,
        )]),
    );
    queue_seeking(&mut unbound, actor);
    unbound.drain_ai_state_change_notifications_for(&sim, &assets, actor);
    assert_consumed(&unbound, actor);

    let mut no_override = EngineInner::new();
    let actor = no_override.add_entity(make_scripted_soldier("NoOverride"));
    let assets = install_state_change_script(&mut no_override, build_scb());
    bind_state_change_actor(&mut no_override, actor, "NoOverride");
    queue_seeking(&mut no_override, actor);
    no_override.drain_ai_state_change_notifications_for(&sim, &assets, actor);
    assert_consumed(&no_override, actor);

    let mut unscripted = EngineInner::new();
    let actor = unscripted.add_entity(make_scripted_soldier(""));
    let assets = install_state_change_script(
        &mut unscripted,
        state_change_scb(vec![state_change_filter_class(
            "StateRecorder",
            false,
            None,
        )]),
    );
    bind_state_change_actor(&mut unscripted, actor, "StateRecorder");
    queue_seeking(&mut unscripted, actor);
    unscripted.drain_ai_state_change_notifications_for(&sim, &assets, actor);
    assert_consumed(&unscripted, actor);
    assert_eq!(
        npc_custom_values(&unscripted, actor)[9],
        4,
        "bound VM does not bypass the owner's is_scripted gate"
    );

    let mut disabled = EngineInner::new();
    let actor = disabled.add_entity(make_scripted_soldier("StateRecorder"));
    let assets = install_state_change_script(
        &mut disabled,
        state_change_scb(vec![state_change_filter_class(
            "StateRecorder",
            false,
            None,
        )]),
    );
    bind_state_change_actor(&mut disabled, actor, "StateRecorder");
    queue_seeking(&mut disabled, actor);
    let mut config = crate::engine::SimConfig::default();
    config.script_enabled = false;
    let disabled_sim = crate::sim_rng::SimulationContext::with_seed_and_config(1, config);
    disabled.drain_ai_state_change_notifications_for(&disabled_sim, &assets, actor);
    assert_consumed(&disabled, actor);
    assert_eq!(
        npc_custom_values(&disabled, actor)[9],
        4,
        "disabled VM did not run"
    );
}
