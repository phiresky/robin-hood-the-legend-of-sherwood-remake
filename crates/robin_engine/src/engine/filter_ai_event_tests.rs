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
//!    source param, so `dispatch_filtered_stimulus(robin, code)` is
//!    allowed (Robin's handle is non-zero) but
//!    `filter_stimulus(…, {source=0}) == false` (blocked).
//!  * Unmapped stimulus type: `filter_stimulus` is a no-op (returns
//!    true) because `stimulus_to_ai_event_code` returns `None`.
//!  * Missing FilterAIEvent override: the base class's implicit
//!    `return 1` must be honoured — `filter_stimulus` returns true
//!    even though `call_actor_function` would otherwise return
//!    `Ok(0)` for "no such function".
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
    let script = MissionScript::from_scb(build_scb()).expect("mission script builds");
    engine.mission_script = Some(script);

    let robin_id = engine.add_entity(make_pc(true));
    let sensitive_id = engine.add_entity(make_scripted_soldier("SourceSensitive"));
    let noov_id = engine.add_entity(make_scripted_soldier("NoOverride"));

    let robin_handle = crate::natives::GameHost::actor_handle(robin_id);
    let sensitive_handle = crate::natives::GameHost::actor_handle(sensitive_id);
    let noov_handle = crate::natives::GameHost::actor_handle(noov_id);

    if let Some(ref mut s) = engine.mission_script {
        assert!(s.bind_actor(sensitive_handle, "SourceSensitive"));
        assert!(s.bind_actor(noov_handle, "NoOverride"));
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
    let (mut engine, robin_handle, sensitive_handle, _) = build_engine();
    // EventView is code 0.  Stimulus carries Robin as the Human
    // source — the stimulus info encodes a 0-based human handle, so
    // `filter_stimulus` will translate it to `robin_handle` before
    // passing to the script.
    let robin_human =
        crate::natives::GameHost::actor_index(robin_handle).expect("valid robin handle") as u32;
    let stim = crate::ai::Stimulus::with_human(crate::ai::StimulusType::EventView, robin_human);

    let allowed = engine.filter_stimulus(&LevelAssets::new(), sensitive_handle, &stim);
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
    let (mut engine, _, sensitive_handle, _) = build_engine();
    // `Stimulus::new` leaves `info = StimulusInfo::None`, so
    // `filter_stimulus` passes source=0.
    let stim = crate::ai::Stimulus::new(crate::ai::StimulusType::EventView);

    let allowed = engine.filter_stimulus(&LevelAssets::new(), sensitive_handle, &stim);
    assert!(!allowed, "source=0 → script returns 0 → block");
}

/// Unmapped stimulus types (engine-only: EventStop, EventEnemyNear,
/// ForceBattleDecision, …) bypass `FilterAIEvent` entirely.  The
/// underlying event code is `-2` for those and no well-formed script
/// branches on `-2`; `stimulus_to_ai_event_code` returns `None` and
/// the filter short-circuits to allow.
#[test]
fn filter_short_circuits_for_unmapped_stimulus_type() {
    let (mut engine, _, sensitive_handle, _) = build_engine();
    // EventEnemyNear is Rust-only; no AI event code mapping.  If the
    // filter actually ran, the script (which returns source) would
    // block `source=0` cases — but we shouldn't reach the script at
    // all for unmapped types.
    let stim = crate::ai::Stimulus::new(crate::ai::StimulusType::EventEnemyNear);

    let allowed = engine.filter_stimulus(&LevelAssets::new(), sensitive_handle, &stim);
    assert!(
        allowed,
        "unmapped stimulus type → skip filter → allow by default"
    );
}

/// Actors with a bound script that doesn't override `FilterAIEvent`
/// inherit the base class's `return 1` default.  `call_actor_function`
/// would otherwise return `Ok(0)` for "no such function"; the
/// `actor_has_function` pre-check prevents that from being misread
/// as a script-blocked stimulus.
#[test]
fn filter_allows_when_actor_has_no_filter_override() {
    let (mut engine, _, _, noov_handle) = build_engine();
    let stim = crate::ai::Stimulus::new(crate::ai::StimulusType::EventView);

    let allowed = engine.filter_stimulus(&LevelAssets::new(), noov_handle, &stim);
    assert!(
        allowed,
        "no FilterAIEvent override → base returns 1 → allow"
    );
}

/// Actors with no bound script instance at all pass through
/// unfiltered.  (Most shipped actors aren't scripted.)
#[test]
fn filter_allows_when_actor_not_bound_to_any_script() {
    let mut engine = EngineInner::new();
    let script = MissionScript::from_scb(build_scb()).expect("mission script builds");
    engine.mission_script = Some(script);

    let unbound_id = engine.add_entity(make_scripted_soldier("SourceSensitive"));
    let unbound_handle = crate::natives::GameHost::actor_handle(unbound_id);

    let stim = crate::ai::Stimulus::new(crate::ai::StimulusType::EventView);
    assert!(
        engine.filter_stimulus(&LevelAssets::new(), unbound_handle, &stim),
        "no bound script → allow"
    );
}

/// `dispatch_filtered_stimulus` should skip `think()` entirely when
/// the filter blocks, and should return `false` to the caller.
#[test]
fn dispatch_returns_false_when_filter_blocks_and_skips_think() {
    let (mut engine, _, sensitive_handle, _) = build_engine();

    // Snapshot AI state pre-dispatch.
    let sensitive_idx =
        crate::natives::GameHost::actor_index(sensitive_handle).expect("valid test handle");
    let before_state = engine.entities[sensitive_idx]
        .as_ref()
        .and_then(|e| e.ai_controller())
        .map(|ai| (ai.current_state, ai.current_substate));

    // EventView with no human info → source=0 → script blocks.
    let stim = crate::ai::Stimulus::new(crate::ai::StimulusType::EventView);
    let ctx = crate::ai::AiContext::default();
    let tick_data = crate::ai::AiPerTickData::stub();
    let sensitive_entity_id = EntityId::Pc(crate::entity_id::PcId(sensitive_idx as u32));

    let handled = engine.dispatch_filtered_stimulus(
        &LevelAssets::new(),
        sensitive_entity_id,
        &stim,
        &ctx,
        &tick_data,
    );
    assert!(!handled, "filter blocked → dispatch returns false");

    // State should be unchanged (think() never ran).
    let after_state = engine.entities[sensitive_idx]
        .as_ref()
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
// (`StopReason::PendingNestedCall` /
// `MissionScript::call_actor_function`'s nested-resume loop).

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

/// Build an SCB with two classes:
///  - `OuterCaller::FilterAIEvent(prototype, source, event)` invokes
///    the `PrototypeFilterEvent` native and returns its result.
///  - `InnerTarget::FilterAIEvent(...)` returns the constant `42`.
fn build_nested_scb() -> ScbFile {
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

    // Inner class.  FilterAIEvent unconditionally returns 42 — a
    // recognisable sentinel that the outer caller can only observe by
    // virtue of the nested dispatch having actually run.
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
    inner_quads.push(q_aff0_iconstant(TMP0, 42));
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

/// Smoke test: an actor script's `FilterAIEvent` calls
/// `PrototypeFilterEvent` on a sibling actor, and the nested call's
/// return value flows back into the outer return.  Verifies that:
///
///  1. The native arm queues a `PendingNestedCall`.
///  2. The interpreter yields with `StopReason::PendingNestedCall`.
///  3. `call_actor_function`'s resume loop dispatches the queued call
///     against the target actor's bound script.
///  4. The result (`42`) is patched into the outer VM's
///     `native_return_value` and read by `Aff1NativeGetReturn`.
///  5. The outer VM resumes and returns the resolved sentinel.
#[test]
fn prototype_filter_event_dispatches_to_target_actor_script() {
    let scb = build_nested_scb();
    let mut script = MissionScript::from_scb(scb).expect("scb builds");

    // Bind two synthetic actor instances.  Their entity handles don't
    // need to map to real engine entities — `call_actor_function`
    // only looks them up in `script.actor_instances`, and our scripts
    // never invoke entity-lookup natives.
    let outer_handle = 1;
    let inner_handle = 2;
    assert!(script.bind_actor(outer_handle, "OuterCaller"));
    assert!(script.bind_actor(inner_handle, "InnerTarget"));

    let result = script
        .call_actor_function(outer_handle, "FilterAIEvent", &[inner_handle, 0, 0])
        .expect("nested dispatch runs cleanly");

    assert_eq!(
        result, 42,
        "outer FilterAIEvent should return the inner target's sentinel \
         (42), proving the nested PrototypeFilterEvent dispatch fired"
    );
}

/// Variant: when `PrototypeFilterEvent` targets a handle with no
/// bound actor script, the recursive `call_actor_function` returns
/// `Ok(0)` — the outer caller observes that as the native's return
/// value.  Note: invoking on a non-scripted prototype in the original
/// engine hit the base-class default (returns 1); our path returns 0
/// because there's no instance at all.  This is a known divergence
/// from the original base default; flagged here so any future
/// "default to 1 when target unbound" decision is explicit.
#[test]
fn prototype_filter_event_unbound_target_returns_zero() {
    let scb = build_nested_scb();
    let mut script = MissionScript::from_scb(scb).expect("scb builds");

    let outer_handle = 1;
    assert!(script.bind_actor(outer_handle, "OuterCaller"));
    // Note: don't bind anyone for handle 99.

    let result = script
        .call_actor_function(outer_handle, "FilterAIEvent", &[99, 0, 0])
        .expect("nested dispatch runs cleanly");

    assert_eq!(
        result, 0,
        "unbound target → call_actor_function returns 0 → outer reads 0"
    );
}
