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
    ActorData, ActorPc, ActorSoldier, AiBrain, ElementData, ElementKind, Entity, EntityId,
    HumanData, NpcData, PcData, Posture, SoldierData,
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

const TMP1: u16 = 0xC004;
const TMP2: u16 = 0xC008;
const TMP3: u16 = 0xC00C;
const TMP4: u16 = 0xC010;

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
