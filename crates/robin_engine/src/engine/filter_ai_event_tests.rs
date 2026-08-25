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
//!  * Class defining no FilterAIEvent: allow, while a missing required VM
//!    remains an error in the shared driver.  Function lookup has no
//!    base-class fallback; such a class is simply never reached by a
//!    filtered stimulus, so allow is the correct verdict.
//!  * Side effects: the filter can observe-and-mutate state each call
//!    (the raison d'être for on-demand vs. precompute).

use crate::coordinates::WorldPoint3D;
use crate::element::{
    ActorCivilian, ActorData, ActorPc, ActorSoldier, AiBrain, CivilianData, ElementBonus,
    ElementData, ElementKind, Entity, EntityId, HumanData, NpcData, ObjectData, ObjectType, PcData,
    Posture, SoldierData,
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
// `NoOverride` — defines no `FilterAIEvent` at all, so the filter has no
// function to call and allows the stimulus.

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

    let mut reject_quads = Vec::new();
    let mut reject_functions = Vec::new();
    for name in [
        "Initialize",
        "ActionChange",
        "HandleEvent",
        "ProcessMessage",
    ] {
        let base = reject_quads.len() as i32;
        let (f, q) = stub_fn(name, base);
        reject_functions.push(f);
        reject_quads.extend(q);
    }
    let filter_addr = reject_quads.len() as i32;
    reject_functions.push(Function {
        name: "FilterAIEvent".into(),
        address: filter_addr,
        num_parameters: 3,
        size_of_return_value: 4,
        size_of_parameters: 12,
        size_of_volatile: 0,
        size_of_temporary: 4,
    });
    reject_quads.push(q_begin_function(0, 1));
    reject_quads.push(q_aff0_iconstant(TMP0, 0));
    reject_quads.push(q_return_val(TMP0));
    reject_quads.push(q_end_function());
    let reject_all = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "RejectAll".into(),
        size_of_member_variables: 0,
        member_variables: vec![],
        functions: reject_functions,
        quads: reject_quads,
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
        classes: vec![startup, source_sensitive, no_override, reject_all],
    }
}

// ───────── Engine fixture ─────────

/// A minimal campaign whose character table backs the PCs built by
/// [`make_pc`]: every test PC carries campaign-description index 0, so the
/// table needs a matching entry at that slot.
fn test_campaign() -> crate::campaign::Campaign {
    crate::campaign::Campaign {
        characters: vec![crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
            ..Default::default()
        }],
        ..Default::default()
    }
}

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
            profile_index: crate::profiles::CharacterProfileIdx(0),
            campaign_description_index: Some(0),
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
    engine.mission_domain.campaign = test_campaign();
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
        std::sync::Arc::make_mut(&mut engine.world.fast_grid),
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

#[test]
fn reentrant_return_to_duty_uses_absent_live_order_not_stale_sprite_animation() {
    let sim = crate::sim_rng::test_context();
    let mut assets = LevelAssets::new();
    let (mut engine, _, _, _) = build_engine();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    let actor = engine
        .world
        .entities
        .npc_ids()
        .find(|&id| {
            engine
                .get_entity(id)
                .and_then(Entity::actor_data)
                .is_some_and(|actor| actor.script_class == "NoOverride")
        })
        .expect("NoOverride soldier");

    {
        let entity = engine.get_entity_mut(actor).expect("NoOverride soldier");
        entity
            .position_iface_mut()
            .set_direction_instantly(crate::position_interface::Direction::from_raw(2));
        entity.element_data_mut().sprite.last_action = crate::order::OrderType::RaisingShield;
        let ai = entity.ai_controller_mut().expect("soldier AI");
        ai.me = actor.index();
        ai.initial_view_direction = 1;
        ai.current_state = crate::ai::AiState::Default;
        ai.current_substate = crate::ai::Substate::DefaultOnPost;
        ai.fire_self_stimulus(crate::ai::StimulusType::EventReturnToDuty);
    }
    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(actor)
            .is_none(),
        "the just-completed animation has no live order even though the sprite retains its row"
    );

    engine.drain_self_stimuli_for_npc(&sim, actor, &assets);
    // The Think boundary only registers the launched Turn with the sequence
    // manager; the manager's own Hourglass dispatches it later in the frame.
    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let ai = engine
        .get_entity(actor)
        .expect("NoOverride soldier")
        .ai_controller()
        .expect("soldier AI");
    assert_eq!(
        ai.current_substate,
        crate::ai::Substate::DefaultGotoPostTurn,
        "GetAnimation() must read NonanimationEnd and take GoTo's already-at-post path"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(actor)
            .and_then(|(sequence, index)| engine
                .orders
                .sequence_manager
                .get_element(sequence, index))
            .map(|element| element.command),
        Some(crate::element::Command::Turn),
        "ReturnToDuty must advance directly to the initial-view Turn, not launch a zero-distance Move"
    );
    assert_eq!(
        engine
            .get_entity(actor)
            .expect("NoOverride soldier")
            .position_iface()
            .get_direction_goal(),
        crate::position_interface::Direction::from_raw(1)
    );
}

#[test]
fn remove_all_subordinates_force_returns_script_locked_civilian_to_duty() {
    let sim = crate::sim_rng::test_context();
    let mut assets = LevelAssets::new();
    let (mut engine, _, _, _) = build_engine();
    let member = engine.add_entity(make_scripted_civilian(""));
    let member_at_post = engine.add_entity(make_scripted_soldier(""));
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    let chief = engine
        .world
        .entities
        .npc_ids()
        .find(|&id| {
            engine
                .get_entity(id)
                .and_then(Entity::actor_data)
                .is_some_and(|actor| actor.script_class == "SourceSensitive")
        })
        .expect("SourceSensitive patrol chief");

    {
        let ai = engine
            .get_entity_mut(member)
            .expect("civilian patrol member")
            .ai_controller_mut()
            .expect("civilian AI");
        ai.me = member.index();
        ai.current_state = crate::ai::AiState::Default;
        ai.current_substate = crate::ai::Substate::DefaultPatrolEnrouteWaiting;
        ai.initial_position = crate::ai::Position {
            x: 100.0,
            ..Default::default()
        };
        ai.script_locked = true;
        ai.patrol_chief = Some(chief);
    }
    engine
        .get_entity_mut(chief)
        .expect("patrol chief")
        .ai_controller_mut()
        .expect("chief AI")
        .theoretical_patrol = vec![member, member_at_post];

    {
        let entity = engine
            .get_entity(member_at_post)
            .expect("on-post civilian patrol member");
        let position = entity.element_data().position();
        let sector = entity.element_data().sector();
        let level = entity.element_data().layer();
        let ai = engine
            .get_entity_mut(member_at_post)
            .expect("on-post civilian patrol member")
            .ai_controller_mut()
            .expect("on-post civilian AI");
        ai.me = member_at_post.index();
        ai.current_state = crate::ai::AiState::Default;
        ai.current_substate = crate::ai::Substate::DefaultOnPost;
        ai.initial_position = crate::ai::Position {
            x: position.x,
            y: position.y,
            sector,
            level,
        };
        ai.patrol_chief = Some(chief);
    }

    let (_, draws) = crate::sim_rng::with_draw_trace(|| {
        engine.script_remove_all_subordinates(&sim, &assets, chief);
    });
    assert!(
        !draws.contains(&crate::sim_rng::RngSite::AiRandomValueRectangle),
        "ClearPatrol's close-post Enemy continuation must not reach GetBoredTime"
    );

    let member_ai = engine
        .get_entity(member)
        .expect("civilian patrol member")
        .ai_controller()
        .expect("civilian AI");
    assert!(
        member_ai.script_locked,
        "ForceReturnToDuty does not release the script lock"
    );
    assert_eq!(member_ai.patrol_chief, None);
    assert_eq!(member_ai.current_state, crate::ai::AiState::Default);
    assert_eq!(
        member_ai.current_substate,
        crate::ai::Substate::DefaultGotoPost,
        "ForceReturnToDuty calls virtual ReturnToDuty directly instead of routing through the lock-refused Think(EVENT_RETURN_TO_DUTY)"
    );
    assert!(
        !member_ai.ai_log.iter().any(|line| {
            line.line_type == crate::ai::LogLineType::EventRefused && line.info == 2
        }),
        "the direct ForceReturnToDuty path must bypass StartThink's script-lock gate"
    );

    let on_post_ai = engine
        .get_entity(member_at_post)
        .expect("on-post civilian patrol member")
        .ai_controller()
        .expect("on-post civilian AI");
    assert_eq!(on_post_ai.patrol_chief, None);
    assert_eq!(
        on_post_ai.current_substate,
        crate::ai::Substate::DefaultGotoPost,
        "ClearPatrol must not recursively complete an already-on-post ForceReturnToDuty"
    );
    assert!(
        on_post_ai.outbox.reentrant.self_stimuli.is_empty(),
        "the suppressed close-post callback must not leak into the member's later owner slot"
    );
}

#[test]
fn remove_all_subordinates_vm_yield_clears_before_following_add_as_subordinate() {
    let sim = crate::sim_rng::test_context();
    let mut assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let old_chief = engine.add_entity(make_scripted_soldier(""));
    let new_chief = engine.add_entity(make_scripted_soldier(""));
    let old_member = engine.add_entity(make_scripted_soldier(""));
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);

    engine
        .get_entity_mut(old_chief)
        .and_then(Entity::ai_controller_mut)
        .expect("old chief has AI")
        .theoretical_patrol = vec![old_member];
    {
        let member_ai = engine
            .get_entity_mut(old_member)
            .and_then(Entity::ai_controller_mut)
            .expect("old member has AI");
        member_ai.patrol_chief = Some(old_chief);
        // Keep this regression focused on the script VM boundary. The
        // default-member ForceReturnToDuty path has dedicated coverage above.
        member_ai.current_state = crate::ai::AiState::Seeking;
    }

    let old_chief_handle = crate::natives::ScriptHandleCodec::actor_handle(old_chief);
    let new_chief_handle = crate::natives::ScriptHandleCodec::actor_handle(new_chief);
    let startup = ClassEntry {
        source_file: "remove_then_add_test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![Function {
            name: "Reassign".into(),
            address: 0,
            num_parameters: 0,
            size_of_return_value: 0,
            size_of_parameters: 0,
            size_of_volatile: 0,
            size_of_temporary: 8,
        }],
        quads: vec![
            q_begin_function(0, 2),
            q_aff0_iconstant(TMP0, old_chief_handle),
            q_native_param(TMP0),
            q_native_call(crate::natives::NativeFn::RemoveAllSubordinates as u32),
            q_aff0_iconstant(TMP0, new_chief_handle),
            q_aff0_iconstant(TMP1, old_chief_handle),
            q_native_param(TMP0),
            q_native_param(TMP1),
            q_native_call(crate::natives::NativeFn::AddAsSubordinate as u32),
            q_return(),
            q_end_function(),
        ],
    };
    engine.scripts.mission = Some(
        MissionScript::from_scb(ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![startup],
        })
        .expect("remove-then-add test SCB builds"),
    );
    engine.attach_script_bindings(&assets);

    engine
        .call_script_vm(
            &sim,
            &assets,
            super::ScriptVmKey::Global,
            "Reassign",
            &[],
            crate::natives::ScriptCallFrame::default(),
        )
        .expect("remove-then-add callback completes");

    assert!(
        engine
            .get_entity(old_chief)
            .and_then(Entity::ai_controller)
            .expect("old chief remains an NPC")
            .theoretical_patrol
            .is_empty(),
        "the old patrol is cleared before the VM continues"
    );
    assert_eq!(
        engine
            .get_entity(new_chief)
            .and_then(Entity::ai_controller)
            .expect("new chief remains an NPC")
            .theoretical_patrol,
        vec![old_chief],
        "the following AddAsSubordinate must observe old_chief.HasPatrol() == false"
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

#[test]
fn review2_result_continuation_consumes_live_script_filter_refusal() {
    use crate::ai::{
        CrossNpcAction, Position, StimulusInfo, StimulusType, ThinkResultContinuation,
    };

    let sim = crate::sim_rng::test_context();
    let (mut engine, _, sensitive_handle, caller_handle) = build_engine();
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    let target = crate::natives::ScriptHandleCodec::actor_handle_index(sensitive_handle)
        .expect("sensitive actor has an entity index") as u32;
    let caller = crate::natives::ScriptHandleCodec::actor_handle_index(caller_handle)
        .expect("caller actor has an entity index") as u32;
    let caller_id = engine
        .entity_id_for_index(caller)
        .expect("script-filter caller exists");
    let target_id = engine
        .entity_id_for_index(target)
        .expect("script-filter target exists");
    engine
        .get_entity_mut(target_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("script-filter target has EnemyAi")
        .base
        .me = target;
    let caller_ai = engine
        .get_entity_mut(caller_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("script-filter caller has EnemyAi");
    caller_ai.base.me = caller;
    caller_ai.alerted_us.clear();
    caller_ai
        .base
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::RequestThinkResult {
            target,
            caller,
            stimulus_type: StimulusType::CallCombatAlert,
            // No Human source makes SourceSensitive::FilterAIEvent return 0.
            info: StimulusInfo::Position(Position::default()),
            continuation: ThinkResultContinuation::OfficerCombatAlertedSoldier {
                last: true,
                use_formation: false,
            },
        });

    engine.drain_direct_ai_owner_boundary(&sim, caller_id, &assets);

    assert!(
        engine
            .get_entity(caller_id)
            .and_then(Entity::enemy_ai)
            .expect("script-filter caller retains EnemyAi")
            .alerted_us
            .is_empty(),
        "a script-authored zero result must prune the candidate"
    );
}

#[test]
fn closure_review_alert_cap_counts_acceptances_after_script_refusals() {
    use crate::ai::{AiState, AlertSoldiersFailureContinuation, Position, Substate};
    use crate::coordinates::MapPoint;
    use crate::profiles::ProfileRank;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    engine.control.frame_counter = 100;
    engine.mission_domain.campaign = test_campaign();
    engine.scripts.mission =
        Some(MissionScript::from_scb(build_scb()).expect("closure-review mission script builds"));

    // Keep the officer and candidates off null human handle zero.
    engine.add_entity(make_pc(true));
    let officer_id = engine.add_entity(make_scripted_soldier(""));
    let mut candidates = Vec::new();
    for index in 0..24 {
        let class = if index < 3 { "RejectAll" } else { "" };
        candidates.push(engine.add_entity(make_scripted_soldier(class)));
    }

    for (index, id) in std::iter::once(officer_id)
        .chain(candidates.iter().copied())
        .enumerate()
    {
        let Entity::Soldier(soldier) = engine
            .get_entity_mut(id)
            .expect("closure-review alert actor exists")
        else {
            panic!("closure-review alert actor changed kind")
        };
        soldier.soldier.cached_camp = crate::element::Camp::Lacklandists;
        soldier.element.active = true;
        soldier
            .element
            .set_position_map(MapPoint::new(index as f32 * 5.0, 0.0));
        soldier.npc.life_points = 50;
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("closure-review alert actor has EnemyAi");
        ai.base.me = id.index();
        ai.soldier_profile_rank = if id == officer_id {
            ProfileRank::Officer
        } else {
            ProfileRank::Soldier
        };
        ai.set_state(AiState::Default, Substate::DefaultOnPost);
    }

    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.attach_script_bindings(&assets);
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut engine.world.entities,
        &mut engine.ai.global,
        std::sync::Arc::make_mut(&mut engine.world.fast_grid),
    );
    let mission = engine
        .scripts
        .mission
        .as_mut()
        .expect("closure-review mission remains loaded");
    for id in candidates.iter().take(3) {
        assert!(mission.bind_actor(
            crate::natives::ScriptHandleCodec::actor_handle(*id),
            "RejectAll",
            &mut engine.script_domains,
            &capabilities,
        ));
    }

    let scratch = engine.build_sim_scratch(&sim, &assets);
    let ctx = crate::engine::ai::build_ai_context_from_entity(
        engine
            .get_entity(officer_id)
            .expect("closure-review officer exists"),
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
    let tick = engine.build_npc_tick_data(&sim, officer_id, &scratch, &assets);
    assert_eq!(tick.camp_soldiers.len(), candidates.len());
    let global = engine.ai.global.clone();
    assert!(
        engine
            .get_entity_mut(officer_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("closure-review officer has EnemyAi")
            .alert_soldiers(
                Position {
                    x: 300.0,
                    ..Default::default()
                },
                0,
                &global,
                None,
                &ctx,
                &tick,
                AlertSoldiersFailureContinuation::None,
            )
    );
    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);

    let officer = engine
        .get_entity(officer_id)
        .and_then(Entity::enemy_ai)
        .expect("closure-review officer retains EnemyAi");
    // Acceptances happen in roster order, but each accepted soldier is
    // inserted into the alerted list sorted by decreasing distance from the
    // officer; candidate distance grows with index here, so the list reads
    // back in reverse roster order.
    let expected: Vec<_> = candidates[3..23]
        .iter()
        .rev()
        .map(|id| id.index())
        .collect();
    assert_eq!(officer.alerted_us, expected);
    assert_eq!(officer.alerted_us.len(), 20);
    assert!(
        !officer.alerted_us.contains(&candidates[23].index()),
        "the scan stops immediately after the twentieth actual acceptance"
    );
    assert!(
        officer.pending_alert_soldier_candidates.is_empty(),
        "unattempted tail candidates are discarded when the accepted cap is reached"
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
    engine.mission_domain.campaign = test_campaign();
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
    engine.world.fast_grid = std::sync::Arc::new(fast_grid);
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
    engine.mission_domain.campaign = test_campaign();
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

fn install_test_wait_timer(
    engine: &mut EngineInner,
    assets: &LevelAssets,
    actor: EntityId,
    frames: u32,
) -> crate::sequence::SequenceId {
    use crate::sequence::{Field, FieldValue};

    let mut element = crate::sequence::SequenceElement::new_generic(
        1,
        crate::element::Command::WaitTimer,
        Some(actor),
    );
    element.set_property(Field::Timer, FieldValue::Integer(frames));
    let sequence = engine.orders.sequence_manager.launch_element(element);
    super::sequence_runtime::WaitCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        next_order_id: &mut engine.orders.next_order_id,
        profiles: &assets.profile_manager,
    }
    .dispatch(actor, crate::element::Command::WaitTimer, sequence, 0);
    let _ = engine
        .orders
        .sequence_manager
        .take_pending_synchronous_actions();
    sequence
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

#[test]
fn unlock_door_done_clears_every_lock_in_owner_slot_with_swapped_creation_order() {
    use crate::element::Command;
    use crate::gate::{Door, DoorIndex};
    use crate::order::{Order, OrderCompletion, OrderType};
    use crate::position_interface::Direction;
    use crate::sequence::{SequenceElement, SequenceState};

    for unlocker_is_earlier in [true, false] {
        let mut engine = EngineInner::new();
        let (unlocker, observer) = if unlocker_is_earlier {
            (
                engine.add_entity(make_pc(true)),
                engine.add_entity(make_pc(false)),
            )
        } else {
            let observer = engine.add_entity(make_pc(false));
            let unlocker = engine.add_entity(make_pc(true));
            (unlocker, observer)
        };
        let assets = LevelAssets::new();
        bind_test_actor_animations(&mut engine, unlocker, &[OrderType::UnlockingDoor]);

        engine.script_domains.interactables.doors.push(Door {
            locked_pc: true,
            locked_npc_villain: true,
            locked_npc_civilian: true,
            unlockable: true,
            ..Door::default()
        });
        let order = Order::new(
            OrderType::UnlockingDoor,
            0.0,
            0.0,
            engine.orders.allocate_order_id(),
        )
        .with_completion(OrderCompletion::UnlockDoor {
            door_id: DoorIndex(0),
        });
        let order_id = order.order_id;
        let mut element = SequenceElement::new_generic(1, Command::UnlockDoor, Some(unlocker));
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

        {
            let entity = engine
                .get_entity_mut(unlocker)
                .expect("unlock owner exists for action-point priming");
            entity
                .position_iface_mut()
                .set_direction_instantly(Direction::NORTH);
            entity.position_iface_mut().set_direction(Direction::EAST);
            let sprite = &mut entity.element_data_mut().sprite;
            sprite.last_processed_order_id = order_id.get();
            sprite.last_action = OrderType::UnlockingDoor;
            sprite.current_row = 0;
            sprite.current_frame = 0;
            sprite.frame_count = 0;
            sprite.action_done_frame = 1;
            sprite.action_done_counter = 0;
        }

        let mut observer_saw_locked = None;
        engine.tick_actor_animation_action_change_slots_with_after_slot(
            &crate::sim_rng::test_context(),
            &assets,
            |engine, owner| {
                if owner == observer {
                    observer_saw_locked =
                        Some(engine.script_domains.interactables.doors[0].locked_pc);
                }
            },
        );

        let door = &engine.script_domains.interactables.doors[0];
        assert!(!door.locked_pc);
        assert!(!door.locked_npc_villain);
        assert!(!door.locked_npc_civilian);
        assert!(!door.unlockable);
        assert_eq!(
            observer_saw_locked,
            Some(!unlocker_is_earlier),
            "only a later creation slot may observe the same-frame lockpick action point"
        );
        assert_eq!(
            engine
                .get_entity(unlocker)
                .expect("unlock owner survives action point")
                .element_data()
                .direction(),
            1,
            "UnlockingDoor must execute the original per-tick Turn()"
        );
        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("unlock sequence remains inspectable at Done");
        assert_eq!(element.state, SequenceState::InProgress);
        assert_eq!(element.orders.len(), 1, "Done must not complete the order");

        engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .expect("unlock sequence remains inspectable after termination")
                .state,
            SequenceState::Terminated,
            "the later Terminated edge must advance and finish the unlock order"
        );
    }
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
    engine.mission_domain.campaign = test_campaign();

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
    engine.mission_domain.campaign = test_campaign();
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
    engine.mission_domain.campaign = test_campaign();
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
fn ai_sequence_launch_drains_immediate_engine_elements_inside_owner_tail() {
    use crate::element::Command;
    use crate::sequence::{Field, FieldValue, Sequence, SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_scripted_soldier(""));
    let assets = LevelAssets::new();

    // A real AI alert sequence can place an engine-immediate element after
    // several actor elements at the same command level.  Sequence::Launch
    // visits all four at once, so the Timer at legacy element index 3 must
    // execute before the derived NPC Hourglass returns.
    let mut sequence = Sequence::new();
    for _ in 0..3 {
        sequence.append_element(SequenceElement::new(1, Command::Generic, Some(owner)));
    }
    let mut timer = SequenceElement::new_generic(1, Command::Timer, None);
    timer.set_property(Field::Timer, FieldValue::Integer(12));
    sequence.append_element(timer);
    engine
        .world
        .entities
        .get_mut(owner)
        .and_then(Entity::ai_controller_mut)
        .expect("AI sequence owner retains its controller")
        .outbox
        .actor
        .launch_sequences
        .push(sequence);

    engine.drain_pending_for_npc(&crate::sim_rng::test_context(), owner, &assets);

    assert_eq!(engine.orders.timer_elements.len(), 1);
    let timer_ref = engine.orders.timer_elements[0].element_ref;
    let timer = engine
        .orders
        .sequence_manager
        .get_element(timer_ref.sequence_id, timer_ref.element_index)
        .expect("AI-launched timer remains inspectable");
    assert_eq!(timer.command, Command::Timer);
    assert_eq!(timer.state, SequenceState::Todo);
    assert!(
        engine
            .orders
            .sequence_manager
            .take_pending_synchronous_actions()
            .is_empty(),
        "the AI owner tail must not leak its inline sequence work"
    );
}

#[test]
fn animation_execution_gates_do_not_skip_action_change() {
    use crate::element::ActionState;
    use crate::order::OrderType;

    for skip in [
        "global-frozen",
        "inactive",
        "execution-frozen",
        "moving",
        "dead",
        "unconscious",
    ] {
        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = test_campaign();
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
                position_direct: true,
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
        // The engine Hourglass runs every element's Hourglass regardless of
        // active state — `active` controls world presence/rendering, not
        // sequence time — so an inactive actor still executes its selected
        // order. Likewise a stale moving action-state cannot suppress
        // ordinary Execute of a selected generic order; only movement
        // elements belong to the movement driver. Original also executes the
        // selected current order for dead and unconscious actors. Only the
        // explicit global and per-actor freeze gates suppress sprite work.
        let expected_last_action = if matches!(skip, "inactive" | "moving" | "dead" | "unconscious")
        {
            OrderType::WalkingUpright
        } else {
            last_action_before
        };
        assert_eq!(
            engine
                .world
                .entities
                .get(actor)
                .expect("skipped actor remains installed")
                .element_data()
                .sprite
                .last_action,
            expected_last_action,
            "{skip} generic sprite execution gate"
        );
    }
}

#[test]
fn movement_owned_token_skip_does_not_sample_stale_execute_inputs() {
    use crate::order::OrderType;

    for movement_order in [OrderType::WalkingUpright, OrderType::WalkingWithSword] {
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
            // An instructed movement stamps the moving action state; only
            // that live state routes the token to the movement driver
            // instead of generic Execute.
            entity
                .actor_data_mut()
                .expect("token-skip actor is typed")
                .action_state = if movement_order == OrderType::WalkingWithSword {
                crate::element::ActionState::MovingSword
            } else {
                crate::element::ActionState::Moving
            };
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
                position_direct: true,
                steps: std::collections::VecDeque::new(),
                triggers_fired: 0,
                current_action: OrderType::Invalid,
                current_reverse: false,
                saved_action_state: None,
            });
        }
        let order =
            crate::order::Order::new(movement_order, 0.0, 0.0, engine.orders.allocate_order_id())
                .with_antagonist(stale);
        // A movement token belongs to the movement driver only when it is
        // carried by a real Movement element; a generic element's order is
        // dispatched through ordinary Execute regardless of action state.
        let mut element = crate::sequence::SequenceElement::new_movement(
            1,
            crate::element::Command::Move,
            Some(actor),
            movement_order,
        );
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

        let (injuries, outcomes, executed) = engine.tick_actor_animation_for(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            actor,
        );

        assert!(injuries.is_empty(), "{movement_order:?}");
        assert!(outcomes.seq_advance.is_empty(), "{movement_order:?}");
        assert!(executed.is_none(), "{movement_order:?}");
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
            "the movement-owned {movement_order:?} token must skip generic Execute without dereferencing stale inputs"
        );
    }
}

#[test]
fn per_actor_wait_initialization_does_not_publish_later_wait_to_earlier_callback() {
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = test_campaign();
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
        crate::order::OrderType::NonanimationEnd as i32,
        "the earlier callback must observe the later actor before that actor's lazy Wait slot: \
         an orderless actor reports the no-animation sentinel, never a published Wait"
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
fn live_actor_walk_visits_callback_spawned_later_slot_and_skips_holes() {
    use super::tick::{ActorOwnerEnvelopePhase as Phase, capture_actor_owner_envelope};

    let mut engine = EngineInner::new();
    let first = engine.add_entity(make_pc(true));
    let removed = engine.add_entity(make_pc(false));
    let later = engine.add_entity(make_pc(false));
    engine.remove_entity(removed);
    let assets = LevelAssets::new();
    let sim = crate::sim_rng::test_context();
    let mut visited = Vec::new();
    let mut spawned = None;

    let (_, phases) = capture_actor_owner_envelope(|| {
        engine.tick_actor_animation_action_change_slots_with_after_slot(
            &sim,
            &assets,
            |engine, owner| {
                visited.push(owner);
                if owner == first {
                    let id = engine.add_entity(make_pc(false));
                    assert!(
                        id.index() > later.index(),
                        "runtime entities are append-only"
                    );
                    spawned = Some(id);
                }
            },
        );
    });
    let spawned = spawned.expect("the first owner's callback must spawn an actor");

    assert_eq!(visited, vec![first, later, spawned]);
    assert_eq!(
        phases
            .into_iter()
            .filter_map(|phase| match phase {
                Phase::BaseActor(owner) => Some(owner),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![first, later, spawned],
        "the live while-slot coordinator must execute the appended actor this frame without shifting across the removed slot"
    );
}

#[test]
fn earlier_owner_callback_installs_invalid_later_pc_init_order_rejected_same_frame() {
    use crate::element::Command;
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let first = engine.add_entity(make_pc(true));
    let later = engine.add_entity(make_pc(false));
    let assets = LevelAssets::new();
    bind_test_actor_animations(&mut engine, first, &[OrderType::WaitingUprightBored]);
    bind_test_actor_animations(&mut engine, later, &[OrderType::Taking]);
    install_test_action(
        &mut engine,
        first,
        OrderType::WaitingUprightBored,
        OrderType::WaitingUprightBored,
    );
    let sim = crate::sim_rng::test_context();
    let mut installed = None;

    engine.tick_actor_animation_action_change_slots_with_after_slot(
        &sim,
        &assets,
        |engine, completed_owner| {
            if completed_owner != first || installed.is_some() {
                return;
            }
            // No antagonist: Original Taking init validity must abort it.
            let mut element = SequenceElement::new(1, Command::Take, Some(later));
            element
                .orders
                .push_back(Order::test_new(OrderType::Taking, 0.0, 0.0));
            let sequence = engine.orders.sequence_manager.launch_element(element);
            engine
                .orders
                .sequence_manager
                .element_in_progress(sequence, 0);
            let _ = engine
                .orders
                .sequence_manager
                .take_pending_synchronous_actions();
            installed = Some(sequence);
        },
    );

    let sequence = installed.expect("earlier callback installed the later PC order");
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("callback-installed element remains inspectable")
            .state,
        SequenceState::Impossible,
        "later PC validity must sample the callback-installed live order at its Execute entry"
    );
}

#[test]
fn terminating_animation_promotes_next_order_before_same_actor_action_change() {
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = test_campaign();
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
    assert_eq!(
        engine
            .world
            .entities
            .get(actor)
            .and_then(|entity| entity.actor_data())
            .expect("terminating actor remains typed")
            .continuation
            .motion_state,
        crate::sprite::MotionState::InProgress,
        "DoNextOrder must rewrite serialized mmotionState when Proceed promotes a successor"
    );
}

#[test]
fn wait_timer_zero_completes_after_execute_and_before_action_change() {
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = test_campaign();
    let actor = engine.add_entity(make_scripted_soldier("ActionObserver"));
    let assets = LevelAssets::new();
    bind_action_observer(&mut engine, &assets, actor);
    bind_test_actor_animations(&mut engine, actor, &[OrderType::WaitingUprightBored]);
    engine
        .world
        .entities
        .get_mut(actor)
        .and_then(|entity| entity.actor_data_mut())
        .expect("timer actor is typed")
        .old_action = OrderType::Pointing;
    let timer_sequence = install_test_wait_timer(&mut engine, &assets, actor, 0);

    engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(timer_sequence, 0)
            .expect("completed timer remains inspectable")
            .state,
        crate::sequence::SequenceState::Terminated
    );
    assert_eq!(
        observed_action_args(&engine, actor),
        (
            OrderType::NonanimationEnd as i32,
            OrderType::Pointing as i32,
        ),
        "zero WAIT_TIMER must Execute its current order, complete/DoNext, then expose the null order to ActionChange"
    );
    assert_eq!(
        engine
            .world
            .entities
            .get(actor)
            .and_then(|entity| entity.actor_data())
            .expect("timer actor remains typed")
            .continuation
            .motion_state,
        crate::sprite::MotionState::Terminated,
        "Actor::Hourglass must retain the post-WAIT_TIMER Execute result in serialized mmotionState"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(actor)
            .is_none(),
        "Original does not create fallback Wait after same-slot DoNextOrder exhaustion"
    );
    assert_eq!(
        engine
            .world
            .entities
            .get(actor)
            .expect("timer actor remains installed")
            .element_data()
            .sprite
            .last_action,
        OrderType::WaitingUprightBored,
        "WAIT_TIMER zero still performs Execute before forcing Terminated"
    );
}

#[test]
fn sequence_manager_instruction_rewrites_terminated_motion_to_in_progress() {
    use crate::element::Command;
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_scripted_soldier(""));
    let assets = LevelAssets::new();
    engine
        .get_entity_mut(actor)
        .and_then(|entity| entity.actor_data_mut())
        .expect("sequence-manager actor is typed")
        .continuation
        .motion_state = crate::sprite::MotionState::Terminated;

    let mut element = SequenceElement::new(1, Command::Generic, Some(actor));
    element.orders.push_back(Order::new(
        OrderType::LookingLeft,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    let successor = engine.orders.sequence_manager.launch_element(element);
    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&crate::sim_rng::test_context(), &mut display, &assets);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(actor),
        Some((successor, 0))
    );
    assert_eq!(
        engine
            .get_entity(actor)
            .and_then(|entity| entity.actor_data())
            .expect("derived-tail actor remains typed")
            .continuation
            .motion_state,
        crate::sprite::MotionState::InProgress,
        "accepted InstructOwner must perform the Original mmotionState rewrite"
    );
}

#[test]
fn accepted_empty_generic_latches_motion_before_immediate_completion() {
    use crate::element::Command;
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_scripted_soldier(""));
    let assets = LevelAssets::new();
    engine
        .get_entity_mut(actor)
        .and_then(|entity| entity.actor_data_mut())
        .expect("sequence-manager actor is typed")
        .continuation
        .motion_state = crate::sprite::MotionState::Terminated;

    let sequence = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::Generic, Some(actor)));
    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&crate::sim_rng::test_context(), &mut display, &assets);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("completed carrier remains inspectable")
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(actor),
        None,
        "empty accepted carrier must complete in the same Instruct call"
    );
    assert_eq!(
        engine
            .get_entity(actor)
            .and_then(|entity| entity.actor_data())
            .expect("sequence-manager actor remains typed")
            .continuation
            .motion_state,
        crate::sprite::MotionState::InProgress,
        "Original latches mmotionState before its empty-order termination"
    );
}

#[test]
fn turning_selects_sprite_row_after_the_direction_step() {
    use crate::element::Command;
    use crate::order::{Order, OrderType};
    use crate::position_interface::Direction;
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_scripted_civilian(""));
    let assets = LevelAssets::new();
    bind_test_actor_animations(&mut engine, actor, &[OrderType::Turning]);

    let order = Order::new(
        OrderType::Turning,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    );
    let order_id = order.order_id;
    let mut element = SequenceElement::new(1, Command::Turn, Some(actor));
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

    {
        let entity = engine
            .get_entity_mut(actor)
            .expect("turning civilian remains installed");
        entity
            .position_iface_mut()
            .set_direction_instantly(Direction::from_raw(0));
        entity
            .position_iface_mut()
            .set_direction(Direction::from_raw(14));
        entity.sprite_mut().last_processed_order_id = order_id.get();
    }

    engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

    let entity = engine
        .get_entity(actor)
        .expect("turning civilian survives its actor slot");
    assert_eq!(u8::from(entity.position_iface().get_direction()), 15);
    assert_eq!(
        entity.sprite().current_row,
        15,
        "Original Turn() advances direction 0->15 before PerformAction selects the directional row"
    );
}

#[test]
fn turning_ignores_stale_sprite_done_while_body_still_rotates() {
    use crate::element::Command;
    use crate::order::{Order, OrderType};
    use crate::position_interface::Direction;
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_scripted_civilian(""));
    let assets = LevelAssets::new();
    bind_test_actor_animations(&mut engine, actor, &[OrderType::Turning]);

    let order = Order::new(
        OrderType::Turning,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    );
    let order_id = order.order_id;
    let mut element = SequenceElement::new(1, Command::Turn, Some(actor));
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

    {
        let entity = engine
            .get_entity_mut(actor)
            .expect("turning civilian remains installed");
        entity
            .position_iface_mut()
            .set_direction_instantly(Direction::from_raw(0));
        entity
            .position_iface_mut()
            .set_direction(Direction::from_raw(14));
        entity.sprite_mut().last_motion_state = Some(MotionState::Done);
        entity.sprite_mut().last_processed_order_id = order_id.get();
    }

    engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("an in-progress Turn must remain live after one rotation step");
    assert_eq!(element.state, SequenceState::InProgress);
    assert!(
        !element
            .current_order()
            .expect("Turn retains its current order")
            .done,
        "the visual sprite's stale Done edge must not complete authoritative Turn motion"
    );
    assert_eq!(
        u8::from(
            engine
                .get_entity(actor)
                .unwrap()
                .position_iface()
                .get_direction()
        ),
        15
    );
    assert_eq!(
        engine.get_entity(actor).unwrap().sprite().last_motion_state,
        Some(MotionState::InProgress),
        "Turn()'s authoritative result must replace the visual sprite edge"
    );

    engine.propagate_done_to_current_orders();
    assert!(
        !engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .current_order()
            .unwrap()
            .done,
        "end-of-frame sprite propagation must preserve the rotating Turn order"
    );
}

#[test]
fn wait_timer_nonzero_preserves_original_extra_zero_frame() {
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_scripted_soldier(""));
    let assets = LevelAssets::new();
    bind_test_actor_animations(&mut engine, actor, &[OrderType::WaitingUprightBored]);
    let timer_sequence = install_test_wait_timer(&mut engine, &assets, actor, 1);
    let sim = crate::sim_rng::test_context();

    engine.tick_actor_animation_action_change_slots(&sim, &assets);
    assert_eq!(
        engine
            .world
            .entities
            .get(actor)
            .and_then(|entity| entity.actor_data())
            .expect("timer actor remains typed")
            .wait_time,
        0
    );
    assert_eq!(
        engine
            .world
            .entities
            .get(actor)
            .and_then(|entity| entity.actor_data())
            .expect("timer actor remains typed")
            .seek_refresh_wait,
        0,
        "WAIT_TIMER countdown updates every Rust mirror of Original's shared mulWaitTime"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(timer_sequence, 0)
            .expect("timer remains installed")
            .state,
        crate::sequence::SequenceState::InProgress,
        "a positive counter is decremented after Execute without completing on the frame it reaches zero"
    );

    engine.tick_actor_animation_action_change_slots(&sim, &assets);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(timer_sequence, 0)
            .expect("timer remains inspectable")
            .state,
        crate::sequence::SequenceState::Terminated,
        "the following Execute observes zero and forces Terminated"
    );
}

#[test]
fn execution_frozen_actor_with_installed_wait_timer_skips_execute_but_completes() {
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_scripted_soldier(""));
    let assets = LevelAssets::new();
    bind_test_actor_animations(&mut engine, actor, &[OrderType::WaitingUprightBored]);
    let timer_sequence = install_test_wait_timer(&mut engine, &assets, actor, 0);
    engine
        .get_entity_mut(actor)
        .and_then(|entity| entity.actor_data_mut())
        .expect("frozen timer owner is an actor")
        .execution_frozen = true;

    engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(timer_sequence, 0)
            .expect("frozen timer remains inspectable")
            .state,
        crate::sequence::SequenceState::Terminated,
        "Actor::Hourglass applies WAIT_TIMER after execution_frozen Execute returns InProgress"
    );
    assert_eq!(
        engine
            .get_entity(actor)
            .expect("frozen timer owner remains installed")
            .element_data()
            .sprite
            .last_action,
        OrderType::NonanimationEnd,
        "RHElementActor::Execute returns before selecting the installed wait animation"
    );
}

#[test]
fn wait_timer_termination_replaces_forwarded_completion_exactly_once() {
    use crate::order::OrderType;
    use crate::sequence::{Field, FieldValue};

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_scripted_soldier(""));
    let assets = LevelAssets::new();
    bind_test_actor_animations(
        &mut engine,
        actor,
        &[OrderType::Pointing, OrderType::Searching],
    );
    let first = crate::order::Order::new(
        OrderType::Pointing,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    );
    let first_id = first.order_id;
    let second = crate::order::Order::new(
        OrderType::Searching,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    );
    let mut element = crate::sequence::SequenceElement::new_generic(
        1,
        crate::element::Command::WaitTimer,
        Some(actor),
    );
    element.set_property(Field::Timer, FieldValue::Integer(0));
    element.orders.extend([first, second]);
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    let _ = engine
        .orders
        .sequence_manager
        .take_pending_synchronous_actions();
    {
        let sprite = &mut engine
            .get_entity_mut(actor)
            .expect("forwarding timer owner exists")
            .element_data_mut()
            .sprite;
        sprite.last_processed_order_id = first_id.get();
        sprite.last_action = OrderType::Pointing;
        sprite.current_row = 0;
        sprite.current_frame = 1;
        sprite.frame_count = 0;
        sprite.action_done_frame = 1;
        sprite.action_done_counter = 0;
    }

    engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("forwarding timer remains inspectable");
    assert_eq!(element.state, crate::sequence::SequenceState::InProgress);
    assert_eq!(
        element.orders.len(),
        1,
        "raw Terminated plus WAIT_TIMER Terminated must not dispatch DoNextOrder twice"
    );
    assert_eq!(
        element
            .orders
            .front()
            .expect("successor remains")
            .order_type,
        OrderType::Searching
    );
}

#[test]
fn same_owner_callback_retargets_execute_termination_to_live_wait_timer() {
    use crate::order::OrderType;
    use crate::sequence::{Field, FieldValue};

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_scripted_soldier(""));
    let assets = LevelAssets::new();
    bind_test_actor_animations(
        &mut engine,
        actor,
        &[OrderType::Pointing, OrderType::Searching],
    );
    let first = crate::order::Order::new(
        OrderType::Pointing,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    );
    let first_id = first.order_id;
    let second = crate::order::Order::new(
        OrderType::Searching,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    );
    let entry_sequence = install_test_order_queue(&mut engine, actor, [first, second]);
    {
        let sprite = &mut engine
            .get_entity_mut(actor)
            .expect("same-owner replacement actor exists")
            .element_data_mut()
            .sprite;
        sprite.last_processed_order_id = first_id.get();
        sprite.last_action = OrderType::Pointing;
        sprite.current_row = 0;
        sprite.current_frame = 1;
        sprite.frame_count = 0;
        sprite.action_done_frame = 1;
        sprite.action_done_counter = 0;
    }
    let mut replacement_sequence = None;

    engine.tick_actor_animation_action_change_slots_with_hooks(
        &crate::sim_rng::test_context(),
        &assets,
        |_, _| {},
        |_, _| {},
        |engine, callback_owner, _, _, _, _, _| {
            if callback_owner != actor || replacement_sequence.is_some() {
                return;
            }
            engine
                .orders
                .sequence_manager
                .postpone_element(entry_sequence, 0);
            let mut timer = crate::sequence::SequenceElement::new_generic(
                1,
                crate::element::Command::WaitTimer,
                Some(actor),
            );
            timer.set_property(Field::Timer, FieldValue::Integer(0));
            let sequence = engine.orders.sequence_manager.launch_element(timer);
            super::sequence_runtime::WaitCommandContext {
                entities: &mut engine.world.entities,
                sequence_manager: &mut engine.orders.sequence_manager,
                next_order_id: &mut engine.orders.next_order_id,
                profiles: &assets.profile_manager,
            }
            .dispatch(actor, crate::element::Command::WaitTimer, sequence, 0);
            let _ = engine
                .orders
                .sequence_manager
                .take_pending_synchronous_actions();
            replacement_sequence = Some(sequence);
        },
        |_, _, _| {},
    );

    let replacement_sequence =
        replacement_sequence.expect("same-owner callback installed replacement timer");
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(replacement_sequence, 0)
            .expect("replacement timer remains inspectable")
            .state,
        crate::sequence::SequenceState::Terminated,
        "effective Terminated must DoNextOrder on the callback-installed live element"
    );
    let entry = engine
        .orders
        .sequence_manager
        .get_element(entry_sequence, 0)
        .expect("Execute-entry element remains inspectable");
    assert_eq!(entry.state, crate::sequence::SequenceState::Postponed);
    assert_eq!(
        entry.orders.len(),
        2,
        "the raw Pointing termination must not retain an old-element completion bucket"
    );
}

#[test]
fn earlier_owner_callback_installs_later_timer_while_reverse_order_defers() {
    use crate::order::OrderType;
    use crate::sequence::{Field, FieldValue};

    for installer_before_target in [true, false] {
        let mut engine = EngineInner::new();
        let first = engine.add_entity(make_scripted_soldier(""));
        let second = engine.add_entity(make_scripted_soldier(""));
        let (installer, target) = if installer_before_target {
            (first, second)
        } else {
            (second, first)
        };
        let assets = LevelAssets::new();
        for actor in [installer, target] {
            bind_test_actor_animations(&mut engine, actor, &[OrderType::WaitingUprightBored]);
            install_test_action(
                &mut engine,
                actor,
                OrderType::WaitingUprightBored,
                OrderType::WaitingUprightBored,
            );
        }
        let sim = crate::sim_rng::test_context();
        let mut timer_sequence = None;

        engine.tick_actor_animation_action_change_slots_with_after_slot(
            &sim,
            &assets,
            |engine, completed_owner| {
                if completed_owner != installer || timer_sequence.is_some() {
                    return;
                }
                let mut timer = crate::sequence::SequenceElement::new_generic(
                    1,
                    crate::element::Command::WaitTimer,
                    Some(target),
                );
                timer.set_property(Field::Timer, FieldValue::Integer(1));
                let sequence = engine.launch_element(timer);
                super::sequence_runtime::WaitCommandContext {
                    entities: &mut engine.world.entities,
                    sequence_manager: &mut engine.orders.sequence_manager,
                    next_order_id: &mut engine.orders.next_order_id,
                    profiles: &assets.profile_manager,
                }
                .dispatch(target, crate::element::Command::WaitTimer, sequence, 0);
                let _ = engine
                    .orders
                    .sequence_manager
                    .take_pending_synchronous_actions();
                timer_sequence = Some(sequence);
            },
        );

        let timer_sequence = timer_sequence.expect("installer callback launched a timer");
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(timer_sequence, 0)
                .expect("callback timer remains installed")
                .state,
            crate::sequence::SequenceState::InProgress
        );
        assert_eq!(
            engine
                .get_entity(target)
                .and_then(|entity| entity.actor_data())
                .expect("callback timer target remains an actor")
                .wait_time,
            if installer_before_target { 0 } else { 1 },
            "only a target whose live creation slot is still ahead may Execute the callback-installed timer this pass"
        );
    }
}

fn waiting_sword_pair(
    attacker_before_defender: bool,
) -> (EngineInner, LevelAssets, EntityId, EntityId) {
    use crate::element::ActionState;
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let first = engine.add_entity(make_scripted_soldier(""));
    let second = engine.add_entity(make_scripted_soldier(""));
    let (attacker, defender) = if attacker_before_defender {
        (first, second)
    } else {
        (second, first)
    };
    for actor in [attacker, defender] {
        bind_test_actor_animations(&mut engine, actor, &[OrderType::WaitingSword]);
    }
    for (actor, x, opponent) in [(attacker, 100.0, defender), (defender, 130.0, attacker)] {
        let entity = engine
            .world
            .entities
            .get_mut(actor)
            .expect("smalltalk actor exists");
        entity
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D {
                x,
                y: 100.0,
                z: 0.0,
            });
        entity
            .element_data_mut()
            .set_sector(crate::position_interface::SectorHandle::new(0));
        entity
            .actor_data_mut()
            .expect("smalltalk actor is typed")
            .action_state = ActionState::WaitingSword;
        entity
            .human_data_mut()
            .expect("smalltalk actor is human")
            .opponents
            .push(opponent);
    }
    {
        let human = engine
            .world
            .entities
            .get_mut(attacker)
            .and_then(|entity| entity.human_data_mut())
            .expect("attacker is human");
        human.smalltalk_initiative = true;
        human.received_smalltalk_initiative = true;
    }
    for actor in [attacker, defender] {
        install_test_action(
            &mut engine,
            actor,
            OrderType::WaitingSword,
            OrderType::WaitingSword,
        );
    }
    let mut assets = LevelAssets::new();
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    profiles.soldiers.push(crate::profiles::SoldierProfile {
        hth_weapon_id: 1,
        ..crate::profiles::SoldierProfile::default()
    });
    profiles
        .hth_weapons
        .push(crate::profiles::HtHWeaponProfile {
            distance: [20, 40, 70, 100],
            ..crate::profiles::HtHWeaponProfile::default()
        });
    (engine, assets, attacker, defender)
}

#[test]
fn waiting_sword_execute_faces_world_xy_not_projected_map_xy() {
    use crate::element::ActionState;
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_scripted_soldier(""));
    let opponent = engine.add_entity(make_scripted_soldier(""));
    bind_test_actor_animations(&mut engine, actor, &[OrderType::WaitingSword]);
    install_test_action(
        &mut engine,
        actor,
        OrderType::WaitingSword,
        OrderType::WaitingSword,
    );
    for (id, position) in [
        (
            actor,
            crate::coordinates::WorldPoint3D {
                x: 100.0,
                y: 100.0,
                z: 0.0,
            },
        ),
        (
            opponent,
            crate::coordinates::WorldPoint3D {
                x: 120.0,
                y: 100.0,
                z: 100.0,
            },
        ),
    ] {
        engine
            .get_entity_mut(id)
            .expect("direction fixture actor exists")
            .element_data_mut()
            .set_position(position);
    }
    let entity = engine
        .get_entity_mut(actor)
        .expect("direction owner exists");
    entity
        .actor_data_mut()
        .expect("direction owner is actor")
        .action_state = ActionState::WaitingSword;
    entity
        .human_data_mut()
        .expect("direction owner is human")
        .opponents
        .push(opponent);

    let _ = engine.tick_actor_animation_for(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        actor,
    );

    let expected = crate::position_interface::vector_to_sector_0_to_15_iso(20.0, 0.0) as u8;
    assert_eq!(
        u8::from(
            engine
                .get_entity(actor)
                .expect("direction owner remains installed")
                .element_data()
                .sprite
                .position_iface
                .get_direction_goal()
        ),
        expected
    );
}

#[test]
fn earlier_smalltalk_hint_is_consumed_by_later_waiting_sword_slot() {
    use crate::element::Command;

    let (mut engine, assets, _attacker, defender) = waiting_sword_pair(true);
    crate::sim_rng::with_seed(1, |sim| {
        engine.tick_actor_animation_action_change_slots(sim, &assets);
    });

    assert!(
        engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(defender, |command| matches!(
                command,
                Command::ParrySmalltalkLeft | Command::ParrySmalltalkRight
            ))
    );
    let human = engine
        .get_entity(defender)
        .and_then(|entity| entity.human_data())
        .expect("defender remains human");
    assert_eq!(human.smalltalk_hint, crate::element::SmalltalkHint::None);
    assert_eq!(human.smalltalk_hint_opponent, None);
}

#[test]
fn frozen_all_keeps_waiting_sword_callbacks_live_without_selecting_sprites() {
    use crate::element::Command;

    let (mut engine, assets, attacker, defender) = waiting_sword_pair(true);
    let before = [attacker, defender].map(|actor| {
        engine
            .get_entity(actor)
            .expect("fighter exists")
            .element_data()
            .sprite
            .last_processed_order_id
    });
    engine.set_actors_frozen(true);
    crate::sim_rng::with_seed(1, |sim| {
        engine.tick_actor_animation_action_change_slots(sim, &assets);
    });

    assert!(
        engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(defender, |command| matches!(
                command,
                Command::ParrySmalltalkLeft | Command::ParrySmalltalkRight
            )),
        "FrozenAll suppresses PerformAction, not WaitingSword's synchronous EvaluateSmalltalkHint/EvaluateSwordfight tail"
    );
    assert_eq!(
        [attacker, defender].map(|actor| {
            engine
                .get_entity(actor)
                .expect("fighter remains live")
                .element_data()
                .sprite
                .last_processed_order_id
        }),
        before,
        "FrozenAll must not stamp either selected sprite order identity"
    );
}

#[test]
fn frozen_all_consumes_actor_initialisation_once_without_sprite_identity() {
    use crate::element::Command;
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_scripted_soldier(""));
    let bottle = engine.add_entity(Entity::Bonus(ElementBonus {
        element: ElementData {
            kind: ElementKind::ObjectOther,
            active: true,
            ..Default::default()
        },
        object: ObjectData {
            object_type: ObjectType::Ale,
            ..Default::default()
        },
    }));
    bind_test_actor_animations(&mut engine, soldier, &[OrderType::DrinkingAle]);
    let mut element =
        SequenceElement::new_interaction(1, Command::DrinkAle, Some(soldier), Some(bottle));
    let mut order = Order::test_new(OrderType::DrinkingAle, 0.0, 0.0);
    order.antagonist = Some(bottle);
    let order_id = order.order_id;
    element.orders.push_back(order);
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    engine.set_actors_frozen(true);
    let assets = LevelAssets::new();

    engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);
    assert_eq!(
        engine
            .get_entity(soldier)
            .unwrap()
            .actor_data()
            .unwrap()
            .last_execute_order_id,
        Some(order_id)
    );
    assert_ne!(
        engine
            .get_entity(soldier)
            .unwrap()
            .sprite()
            .last_processed_order_id,
        order_id.get()
    );

    engine
        .get_entity_mut(bottle)
        .unwrap()
        .element_data_mut()
        .active = false;
    engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::InProgress,
        "DrinkingAle's inactive-antagonist IsInitialisation gate must not repeat on a later frozen tick"
    );
}

#[test]
fn frozen_all_runs_weak_sword_actor_initialisation_before_sprite_start() {
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let weak = engine.add_entity(make_pc(true));
    let opponent = engine.add_entity(make_pc(false));
    bind_test_actor_animations(&mut engine, weak, &[OrderType::BeingWeakSword]);
    install_test_action(
        &mut engine,
        weak,
        OrderType::BeingWeakSword,
        OrderType::WaitingSword,
    );
    engine
        .get_entity_mut(weak)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents = vec![opponent];
    engine
        .get_entity_mut(opponent)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents = vec![weak];
    engine
        .get_entity_mut(weak)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .smalltalk_initiative = true;
    engine.set_actors_frozen(true);

    engine.tick_actor_animation_action_change_slots(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
    );

    assert!(
        !engine
            .get_entity(weak)
            .unwrap()
            .human_data()
            .unwrap()
            .smalltalk_initiative
    );
    let opponent_human = engine.get_entity(opponent).unwrap().human_data().unwrap();
    assert!(opponent_human.smalltalk_initiative);
    assert!(opponent_human.received_smalltalk_initiative);
    assert_eq!(
        engine
            .get_entity(weak)
            .unwrap()
            .sprite()
            .last_processed_order_id,
        u32::MAX,
        "weak/stunned IsInitialisation is actor-owned and precedes the frozen sprite boundary"
    );
}

#[test]
fn frozen_all_stunned_sword_initialisation_preserves_smalltalk_initiative() {
    use crate::order::OrderType;
    use crate::titbit::{ElementHandle, TitbitKind};

    let mut engine = EngineInner::new();
    let stunned = engine.add_entity(make_pc(true));
    let opponent = engine.add_entity(make_pc(false));
    bind_test_actor_animations(&mut engine, stunned, &[OrderType::BeingStunnedSword]);
    install_test_action(
        &mut engine,
        stunned,
        OrderType::BeingStunnedSword,
        OrderType::WaitingSword,
    );
    {
        let human = engine
            .get_entity_mut(stunned)
            .unwrap()
            .human_data_mut()
            .unwrap();
        human.opponents = vec![opponent];
        human.smalltalk_initiative = true;
    }
    engine
        .get_entity_mut(opponent)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents = vec![stunned];
    engine.set_actors_frozen(true);

    engine.tick_actor_animation_action_change_slots(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
    );

    assert!(
        engine
            .get_entity(stunned)
            .unwrap()
            .human_data()
            .unwrap()
            .smalltalk_initiative,
        "BeingStunnedSword must not perform ExecuteWeakness's initiative handoff"
    );
    let opponent_human = engine.get_entity(opponent).unwrap().human_data().unwrap();
    assert!(!opponent_human.smalltalk_initiative);
    assert!(!opponent_human.received_smalltalk_initiative);
    assert!(
        engine
            .feedback
            .titbit_manager
            .titbit_exists(TitbitKind::WeakStunned, ElementHandle(stunned.index()))
    );
}

#[test]
fn stunned_sword_initialisation_dispatches_adversary_weak_synchronously() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    // AI HumanHandle zero is the legacy null sentinel.
    let _sentinel_slot = engine.add_entity(make_pc(false));
    let stunned = engine.add_entity(make_pc(true));
    let opponent = engine.add_entity(make_scripted_soldier(""));
    bind_test_actor_animations(&mut engine, stunned, &[OrderType::BeingStunnedSword]);
    install_test_action(
        &mut engine,
        stunned,
        OrderType::BeingStunnedSword,
        OrderType::WaitingSword,
    );
    engine
        .get_entity_mut(stunned)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents = vec![opponent];
    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(opponent).unwrap() else {
            unreachable!()
        };
        soldier.human.opponents = vec![stunned];
        soldier.npc.view_radius = 400;
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = opponent.index();
        ai.base.primary_target = stunned.index();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfight;
        // An unrelated deferred detection event must not be stolen by the
        // direct combat callback.
        ai.base
            .outbox
            .detection
            .stimuli
            .push(Stimulus::new(StimulusType::EventFitAgain));
    }
    engine.control.frame_counter = 42;
    engine.set_actors_frozen(true);
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);

    engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

    let opponent_ai = engine.get_entity(opponent).unwrap().enemy_ai().unwrap();
    assert!(
        opponent_ai.base.timer_is_running,
        "EVENT_ADVERSARY_WEAK must enter ReconsiderSwordfight before actor initialization returns"
    );
    assert_eq!(opponent_ai.base.when_does_timer_ring, 62);
    assert_eq!(
        opponent_ai
            .base
            .outbox
            .detection
            .stimuli
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![StimulusType::EventFitAgain],
        "the direct weak callback must close immediately while preserving older deferred stimuli"
    );
}

#[test]
fn civilian_random_speech_closes_its_owner_boundary_before_the_lock_gate() {
    use crate::engine::types::SimulationRng;
    use crate::profiles::CivilianType;

    let mut engine = EngineInner::new();
    let beggar = engine.add_entity(make_scripted_civilian(""));
    if let Entity::Civilian(civilian) = engine.get_entity_mut(beggar).unwrap() {
        civilian.civilian.cached_civilian_type = CivilianType::Beggar;
        civilian.npc.register_number = 0;
        civilian.npc.ai_brain.base_mut().unwrap().me = beggar.index();
    }
    engine.control.frame_counter = 100;
    // Gate succeeds and choice 2 selects CivBeggarBegging, matching the
    // frame-1381 Original call shape.
    engine.control.rng = SimulationRng::with_original_replay(vec![915_892_857, 378_770_797]);
    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .civilians
        .push(crate::profiles::CivilianProfile {
            civilian_type: CivilianType::Beggar,
            ..Default::default()
        });

    engine.with_simulation_context(|engine, sim| {
        engine.tick_civilian_random_speech_for_npc(sim, beggar, &assets);
    });

    let owner_work = &engine
        .get_entity(beggar)
        .unwrap()
        .ai_controller()
        .unwrap()
        .outbox
        .reentrant
        .owner_work;
    assert!(
        owner_work.is_empty(),
        "RandomSpeech's synchronous Original Say call must settle before the following lock gate"
    );
    assert_eq!(engine.control.rng.original_replay_cursor(), Some(2));
}

#[test]
fn actor_execute_arm_ledger_has_unique_linked_routes() {
    use super::tick::ORIGINAL_ACTOR_EXECUTE_CATALOG;

    assert_eq!(ORIGINAL_ACTOR_EXECUTE_CATALOG.len(), 308);
    let mut expected = std::collections::HashSet::new();
    for &(override_kind, order, owner) in ORIGINAL_ACTOR_EXECUTE_CATALOG {
        assert!(
            expected.insert((override_kind, order)),
            "duplicate catalog arm {override_kind:?}/{order:?}"
        );
        assert_eq!(
            super::tick::classify_actor_execute_arm(override_kind, order),
            Some(owner)
        );
        super::tick::assert_execute_owner_handler_is_linked(owner);
    }
}

#[test]
fn every_active_ability_order_routes_to_the_production_ability_owner() {
    use super::tick::{ExecuteOwnerFamily, classify_live_actor_execute_arm};
    use crate::entity_id::{CivilianId, PcId};
    use crate::movement::AbilityKind;
    use crate::order::OrderType;

    let pc = EntityId::Pc(PcId(0));
    for kind in AbilityKind::ALL {
        let assert_route = |actor, order| {
            assert_eq!(
                classify_live_actor_execute_arm(actor, order),
                Some(ExecuteOwnerFamily::Ability),
                "{kind:?} phase {order:?} is not routed to its production owner"
            );
        };
        match kind {
            AbilityKind::Listen => {
                for order in [
                    OrderType::TransitionWaitingUprightListening,
                    OrderType::Listening,
                    OrderType::TransitionListeningWaitingUpright,
                ] {
                    assert_route(pc, order);
                }
            }
            AbilityKind::ReceivePurse => {
                for order in [
                    OrderType::ReceivingPurse,
                    OrderType::WaitingWithPurse,
                    OrderType::TransitionWaitingWithPurseWaitingUpright,
                ] {
                    assert_route(EntityId::Civilian(CivilianId(0)), order);
                }
            }
            AbilityKind::Heal => {
                assert_route(pc, OrderType::Healing);
                assert_route(pc, OrderType::Eating);
            }
            other => assert_route(pc, crate::abilities::ability_order_type(other)),
        }
    }
}

#[test]
fn every_canonical_active_bow_order_routes_to_the_production_bow_owner() {
    use super::tick::{ExecuteOwnerFamily, classify_live_actor_execute_arm};
    use crate::entity_id::{PcId, SoldierId};
    use crate::order::OrderType;

    for &order in crate::bow_shot::ACTIVE_BOW_ORDERS {
        let actor = if matches!(
            order,
            OrderType::ShootingWithBowLeaningOut
                | OrderType::TransitionRaisingBowLeaningOut
                | OrderType::TransitionLoweringBowLeaningOut
        ) {
            EntityId::Soldier(SoldierId(0))
        } else {
            EntityId::Pc(PcId(0))
        };
        assert_eq!(
            classify_live_actor_execute_arm(actor, order),
            Some(ExecuteOwnerFamily::Bow),
            "canonical active bow phase {order:?} is not routed to Bow"
        );
    }
}

#[test]
fn every_specialized_melee_and_beggar_order_routes_to_its_production_owner() {
    use super::tick::{ExecuteOwnerFamily, MELEE_ORDERS, classify_live_actor_execute_arm};
    use crate::entity_id::PcId;
    use crate::order::OrderType;

    let pc = EntityId::Pc(PcId(0));
    for &order in MELEE_ORDERS {
        assert_eq!(
            classify_live_actor_execute_arm(pc, order),
            Some(ExecuteOwnerFamily::Melee),
            "active melee order {order:?} is not routed to Melee"
        );
    }
    assert_eq!(
        classify_live_actor_execute_arm(pc, OrderType::SimulatingBeggar),
        Some(ExecuteOwnerFamily::Beggar)
    );
}

#[test]
fn smalltalk_strikes_route_to_their_distinct_human_execute_arm() {
    use super::tick::{ExecuteOwnerFamily, classify_live_actor_execute_arm};
    use crate::entity_id::{PcId, SoldierId};
    use crate::order::OrderType;

    for actor in [EntityId::Pc(PcId(0)), EntityId::Soldier(SoldierId(0))] {
        for order in [
            OrderType::StrikingLeftSmalltalk,
            OrderType::StrikingRightSmalltalk,
            OrderType::StrikingLowLeftSmalltalk,
            OrderType::StrikingLowRightSmalltalk,
        ] {
            assert_eq!(
                classify_live_actor_execute_arm(actor, order),
                Some(ExecuteOwnerFamily::GenericAnimation),
                "smalltalk strike {order:?} must retain its bespoke Human::Execute semantics"
            );
        }
    }
}

#[test]
fn later_smalltalk_hint_defers_for_already_visited_defender() {
    use crate::element::Command;

    let (mut engine, assets, _attacker, defender) = waiting_sword_pair(false);
    crate::sim_rng::with_seed(1, |sim| {
        engine.tick_actor_animation_action_change_slots(sim, &assets);
    });

    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(defender, |command| matches!(
                command,
                Command::ParrySmalltalkLeft | Command::ParrySmalltalkRight
            ))
    );
    assert_ne!(
        engine
            .get_entity(defender)
            .and_then(|entity| entity.human_data())
            .expect("defender remains human")
            .smalltalk_hint,
        crate::element::SmalltalkHint::None,
        "the later attacker mutates the already-visited defender, but the defender cannot consume the hint until its next slot"
    );
}

#[test]
fn earlier_initiative_transfer_drives_later_recipient_slot() {
    use crate::element::Command;

    let (mut engine, assets, attacker, defender) = waiting_sword_pair(true);
    {
        let human = engine
            .get_entity_mut(attacker)
            .and_then(|entity| entity.human_data_mut())
            .expect("attacker remains human");
        human.received_smalltalk_initiative = false;
        human.relative_fighting_ability = 100;
    }
    crate::sim_rng::with_seed(9, |sim| {
        engine.tick_actor_animation_action_change_slots(sim, &assets);
    });

    assert!(
        engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(defender, |command| matches!(
                command,
                Command::SwordstrikeSmalltalkLeft | Command::SwordstrikeSmalltalkRight
            ))
    );
}

#[test]
fn later_initiative_transfer_cannot_reenter_visited_recipient_slot() {
    use crate::element::Command;

    let (mut engine, assets, attacker, defender) = waiting_sword_pair(false);
    {
        let human = engine
            .get_entity_mut(attacker)
            .and_then(|entity| entity.human_data_mut())
            .expect("attacker remains human");
        human.received_smalltalk_initiative = false;
        human.relative_fighting_ability = 100;
    }
    crate::sim_rng::with_seed(9, |sim| {
        engine.tick_actor_animation_action_change_slots(sim, &assets);
    });

    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(defender, |command| matches!(
                command,
                Command::SwordstrikeSmalltalkLeft | Command::SwordstrikeSmalltalkRight
            ))
    );
    let human = engine
        .get_entity(defender)
        .and_then(|entity| entity.human_data())
        .expect("defender remains human");
    assert!(human.smalltalk_initiative);
    assert!(human.received_smalltalk_initiative);
}

#[test]
fn earlier_opponent_prune_synchronously_quits_both_combatants() {
    use crate::element::ActionState;
    use crate::order::OrderType;

    for pruner_before_mutated in [true, false] {
        let mut engine = EngineInner::new();
        let first = engine.add_entity(make_scripted_soldier(""));
        let second = engine.add_entity(make_scripted_soldier(""));
        let (pruner, mutated) = if pruner_before_mutated {
            (first, second)
        } else {
            (second, first)
        };
        for actor in [pruner, mutated] {
            bind_test_actor_animations(&mut engine, actor, &[OrderType::WaitingSword]);
            install_test_action(
                &mut engine,
                actor,
                OrderType::WaitingSword,
                OrderType::WaitingSword,
            );
            engine
                .get_entity_mut(actor)
                .and_then(|entity| entity.actor_data_mut())
                .expect("opponent-prune fighter is typed")
                .action_state = ActionState::WaitingSword;
            engine
                .get_entity_mut(actor)
                .and_then(Entity::enemy_ai_mut)
                .expect("opponent-prune fighter has enemy AI")
                .hth_weapon_id = 1;
        }
        for (actor, x, z, sector) in [(pruner, 100.0, 0.0, 1), (mutated, 130.0, 41.0, 2)] {
            let element = engine
                .get_entity_mut(actor)
                .expect("opponent-prune fighter exists")
                .element_data_mut();
            element.set_position(crate::coordinates::WorldPoint3D { x, y: 100.0, z });
            element.set_sector(crate::position_interface::SectorHandle::new(sector));
        }
        engine
            .get_entity_mut(pruner)
            .and_then(|entity| entity.human_data_mut())
            .expect("pruner is human")
            .opponents = vec![mutated];
        {
            let human = engine
                .get_entity_mut(mutated)
                .and_then(|entity| entity.human_data_mut())
                .expect("mutated fighter is human");
            human.opponents = vec![pruner];
            human.smalltalk_initiative = true;
            human.received_smalltalk_initiative = true;
        }
        let mut assets = LevelAssets::new();
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles.soldiers.push(crate::profiles::SoldierProfile {
            hth_weapon_id: 1,
            ..crate::profiles::SoldierProfile::default()
        });
        profiles
            .hth_weapons
            .push(crate::profiles::HtHWeaponProfile {
                distance: [20, 40, 70, 100],
                ..crate::profiles::HtHWeaponProfile::default()
            });
        crate::sim_rng::with_seed(5, |sim| {
            engine.tick_actor_animation_action_change_slots(sim, &assets);
        });
        // The prune launches both QuitSwordfight elements at the owner slot,
        // but a normal-priority launch is only registered with the sequence
        // manager; the manager's own Hourglass — which runs after every
        // element Hourglass in the frame — dispatches and completes them.
        let mut display = crate::engine::HostDisplayState::default();
        crate::sim_rng::with_seed(5, |sim| {
            engine.hourglass_phase_sequences(sim, &mut display, &assets);
        });
        for actor in [pruner, mutated] {
            // QuitSwordfight is a real animated command: the manager
            // Hourglass instructs it this frame, its lower-sword transition
            // then plays out over the following frames before the action
            // state settles back to Waiting.
            assert_eq!(
                engine
                    .orders
                    .sequence_manager
                    .current_element_for_actor(actor)
                    .and_then(|(sequence, index)| engine
                        .orders
                        .sequence_manager
                        .get_element(sequence, index))
                    .map(|element| (element.command, element.state)),
                Some((
                    crate::element::Command::QuitSwordfight,
                    crate::sequence::SequenceState::InProgress,
                )),
                "both QuitSwordfight launches are instructed by the same frame's manager Hourglass"
            );
            assert_eq!(
                engine
                    .get_entity(actor)
                    .and_then(|entity| entity.actor_data())
                    .expect("pruned fighter remains an actor")
                    .action_state,
                ActionState::WaitingSword,
                "the sword action state persists until the quit transition finishes"
            );
            assert!(
                engine
                    .get_entity(actor)
                    .and_then(|entity| entity.human_data())
                    .expect("pruned fighter remains human")
                    .opponents
                    .is_empty()
            );
        }
    }
}

#[test]
fn skipped_and_non_waiting_sword_slots_do_not_touch_combat_refs_or_rng() {
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let skipped = engine.add_entity(make_scripted_soldier(""));
    let ordinary = engine.add_entity(make_scripted_soldier(""));
    let stale = engine.add_entity(make_scripted_soldier(""));
    engine.remove_entity(stale);
    for actor in [skipped, ordinary] {
        engine
            .world
            .entities
            .get_mut(actor)
            .and_then(|entity| entity.human_data_mut())
            .expect("test actor is human")
            .opponents
            .push(stale);
    }
    engine
        .world
        .entities
        .get_mut(skipped)
        .and_then(|entity| entity.actor_data_mut())
        .expect("skipped actor is typed")
        .execution_frozen = true;
    bind_test_actor_animations(&mut engine, skipped, &[OrderType::WaitingSword]);
    bind_test_actor_animations(&mut engine, ordinary, &[OrderType::WaitingUpright]);
    install_test_action(
        &mut engine,
        skipped,
        OrderType::WaitingSword,
        OrderType::WaitingSword,
    );
    install_test_action(
        &mut engine,
        ordinary,
        OrderType::WaitingUpright,
        OrderType::WaitingUpright,
    );
    let assets = LevelAssets::new();

    let observed = crate::sim_rng::with_seed(77, |sim| {
        engine.tick_actor_animation_action_change_slots(sim, &assets);
        crate::sim_rng::bool(sim, crate::sim_rng::RngSite::SmalltalkStrikeSide)
    });
    let expected = crate::sim_rng::with_seed(77, |sim| {
        crate::sim_rng::bool(sim, crate::sim_rng::RngSite::SmalltalkStrikeSide)
    });
    assert_eq!(observed, expected);
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
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    // The target starts unconscious, so the shared fixture skips its combat
    // attachments — but it wakes up mid-test and then needs a live HtH
    // weapon profile for its fighter snapshot.
    engine
        .get_entity_mut(target)
        .and_then(Entity::enemy_ai_mut)
        .expect("wake target has enemy AI")
        .hth_weapon_id = 1;
    (engine, assets, rescuer, target)
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
fn waking_up_done_does_not_force_awake_a_script_locked_npc() {
    let (mut engine, assets, _rescuer, target) = waking_up_creation_order_engine(true);
    engine
        .get_entity_mut(target)
        .and_then(Entity::ai_controller_mut)
        .expect("wake target has AI")
        .script_locked = true;

    engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

    let target_entity = engine
        .world
        .entities
        .get(target)
        .expect("script-locked wake target remains installed");
    assert!(
        target_entity
            .human_data()
            .expect("wake target is human")
            .unconscious,
        "Original SetConcussionOfTheBrain clamps a script-locked NPC at the wake threshold"
    );
    assert!(
        !target_entity
            .ai_controller()
            .expect("wake target retains AI")
            .outbox
            .detection
            .stimuli
            .iter()
            .any(|stimulus| stimulus.stimulus_type == crate::ai::StimulusType::EventFitAgain),
        "a target that did not wake must not receive EVENT_FITAGAIN"
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
    engine.mission_domain.campaign = test_campaign();
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
        std::sync::Arc::make_mut(&mut engine.world.fast_grid),
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

fn install_unrelated_multi_exit_building_actor(engine: &mut EngineInner) -> EntityId {
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
        position_direct: true,
        steps: VecDeque::new(),
        triggers_fired: 0,
        current_action: crate::order::OrderType::default(),
        current_reverse: false,
        saved_action_state: None,
    });
    pc.actor.passing_door_directly = true;
    pc.element
        .sprite
        .position_iface
        .set_door_for_test(crate::position_interface::DoorHandle(0));

    let building_sector = SectorNumber::new(8);
    engine.script_domains.interactables.doors = vec![
        Door {
            door_type: DoorType::Building,
            sector_out: SectorNumber::new(7),
            sector_in: building_sector,
            sector_out_index: crate::fast_find_grid::SectorIndex::new(1),
            sector_in_index: crate::fast_find_grid::SectorIndex::new(0),
            point_out: crate::coordinates::MapPoint::new(0.0, 0.0),
            point_in: crate::coordinates::MapPoint::new(10.0, 0.0),
            ..Door::default()
        },
        Door {
            door_type: DoorType::Building,
            sector_out: SectorNumber::new(9),
            sector_in: building_sector,
            sector_out_index: crate::fast_find_grid::SectorIndex::new(2),
            sector_in_index: crate::fast_find_grid::SectorIndex::new(0),
            point_out: crate::coordinates::MapPoint::new(100.0, 0.0),
            point_in: crate::coordinates::MapPoint::new(90.0, 0.0),
            ..Door::default()
        },
    ];
    let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
    // Include the probe owner's ordinary sector as well. Once this helper
    // installs an exact arena, every live public sector used by the fixture
    // must remain resolvable; otherwise building the owner view correctly
    // rejects the synthetic arena as incomplete.
    for (index, sector_number) in [
        building_sector,
        SectorNumber::new(7),
        SectorNumber::new(9),
        SectorNumber::new(1),
    ]
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
    door_actor
}

fn select_unrelated_pass_door_fixture(engine: &mut EngineInner, door_actor: EntityId) {
    use crate::gate::DoorIndex;

    // `RHActor::IsPassingDoor` is selected-command state, not merely the
    // actor's retained door choreography. Keep this unrelated control actor
    // frozen so the synthetic PassDoor remains selected during fused owner
    // walks without trying to execute the intentionally minimal fixture.
    engine
        .get_entity_mut(door_actor)
        .expect("unrelated door-passing actor exists")
        .actor_data_mut()
        .expect("unrelated door-passing actor has actor data")
        .execution_frozen = true;
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
        *gate_id = Some(DoorIndex(0));
        *direction = 1;
    } else {
        unreachable!("PassDoor fixture must be a movement element")
    }
    let pass_sequence = engine.orders.sequence_manager.launch_element(pass);
    engine
        .orders
        .sequence_manager
        .element_in_progress(pass_sequence, 0);
}

#[test]
fn destination_forecast_ignores_a_stale_door_pass_without_a_live_door() {
    use crate::element::ActiveDoorPass;
    use crate::gate::DoorIndex;
    use std::collections::VecDeque;

    let mut actor = make_pc(true);
    let Entity::Pc(pc) = &mut actor else {
        unreachable!("PC fixture changed kind")
    };
    pc.actor.active_door_pass = Some(ActiveDoorPass {
        door_index: DoorIndex(7),
        direct: true,
        position_direct: true,
        steps: VecDeque::new(),
        triggers_fired: 0,
        current_action: crate::order::OrderType::WalkingUpright,
        current_reverse: false,
        saved_action_state: None,
    });

    assert_eq!(
        super::ai::extract_forecast_input(&actor, true)
            .expect("actor has forecast state")
            .door_pass,
        None,
        "Original ForecastDestinationForIA falls back when GetDoor() is NULL"
    );
    assert!(
        !super::ai::extract_forecast_input(&actor, true)
            .expect("actor has forecast state")
            .passing_door_directly,
        "a stale passage mirror must not manufacture the independent direct-passage latch"
    );

    let Entity::Pc(pc) = &mut actor else {
        unreachable!("PC fixture changed kind")
    };
    pc.element
        .sprite
        .position_iface
        .set_door_for_test(crate::position_interface::DoorHandle(7));
    assert_eq!(
        super::ai::extract_forecast_input(&actor, true)
            .expect("actor has forecast state")
            .door_pass,
        Some((DoorIndex(7), false)),
        "the live door must use the independent serialized passage-direction latch, not the runtime mirror"
    );
}

#[test]
fn destination_forecast_uses_legacy_saved_live_door_without_runtime_pass() {
    use crate::element::Command;
    use crate::entity_id::PcId;
    use crate::gate::DoorIndex;
    use crate::sequence::{SequenceElement, SequenceManager};

    let mut actor = make_pc(true);
    let Entity::Pc(pc) = &mut actor else {
        unreachable!("PC fixture changed kind")
    };
    assert!(
        pc.actor.active_door_pass.is_none(),
        "legacy adoption does not reconstruct runtime door choreography"
    );
    pc.actor.passing_door_directly = true;
    pc.element
        .sprite
        .position_iface
        .set_door_for_test(crate::position_interface::DoorHandle(133));

    let owner = EntityId::Pc(PcId(0));
    let mut sequences = SequenceManager::new();
    assert!(
        !super::ai::selected_actor_is_passing_door(&sequences, owner),
        "a live saved door outside selected PassDoor must use the current-position forecast"
    );
    assert_eq!(
        super::ai::extract_forecast_input(&actor, false)
            .expect("actor has forecast state")
            .door_pass,
        None
    );

    let sequence_id =
        sequences.launch_element(SequenceElement::new(1, Command::PassDoor, Some(owner)));
    sequences.element_in_progress(sequence_id, 0);
    let selected_pass_door = super::ai::selected_actor_is_passing_door(&sequences, owner);
    assert!(selected_pass_door);

    let input = super::ai::extract_forecast_input(&actor, selected_pass_door)
        .expect("actor has forecast state");
    assert_eq!(input.door_pass, Some((DoorIndex(133), true)));
    assert!(input.passing_door_directly);
}

#[test]
fn destination_forecast_retains_direct_passage_after_the_live_door_clears() {
    let mut actor = make_pc(true);
    let Entity::Pc(pc) = &mut actor else {
        unreachable!("PC fixture changed kind")
    };
    pc.actor.passing_door_directly = true;

    let input = super::ai::extract_forecast_input(&actor, true).expect("actor has forecast state");
    assert_eq!(input.door_pass, None);
    assert!(
        input.passing_door_directly,
        "Original keeps mbPassingDoorDirectly after PassDoor clears GetDoor()"
    );
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
    // SeekArea's causal Move is only registered with the sequence manager;
    // normal-priority launches are dispatched by the manager's Hourglass
    // later in the frame, so an adjacent GetCurrentAction still reads the
    // orderless no-animation sentinel.
    assert_eq!(
        seeking_values[3],
        crate::order::OrderType::NonanimationEnd as i32,
        "SeekArea's GoTo Move stays registered-not-instructed at the adjacent instruction"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.owner == Some(seeking)
                    && element.command == crate::element::Command::Move
                    && element.state == crate::sequence::SequenceState::Todo
            }),
        "the causal SeekArea Move is queued for the manager Hourglass"
    );

    engine.ai.global.seek_points[0].locked = false;
    run_ai_state_native_probe(&mut engine, &assets, seeking_filter_zero);
    let zero_values = npc_custom_values(&engine, seeking_filter_zero);
    // This actor is 90 map units from the shared seek point, but the
    // deterministic acceptance draw rejects that point. Original SeekArea
    // appends personal point 2222 at its center when the list is empty, so
    // GoTo receives the actor's current position and EndThink synchronously
    // reaches FilterAIEvent(EVENT_REACHPOINT). The zero return refuses that
    // recursive Think after the earlier Seeking SetState callback.
    assert_eq!(
        &zero_values[0..3],
        &[3, 3, 3],
        "the refused REACHPOINT filter follows the synchronous Seeking state callback"
    );
    assert_eq!(
        zero_values[3],
        crate::order::OrderType::NonanimationEnd as i32,
        "the center fallback reaches its destination without launching a Move"
    );
    let zero_ai = engine
        .world
        .entities
        .get(seeking_filter_zero)
        .and_then(Entity::enemy_ai)
        .expect("filter-zero soldier retains Enemy AI");
    assert_eq!(zero_ai.actual_seek_point, Some(2222));
    assert_eq!(
        zero_ai
            .personal_seek_point_2
            .as_ref()
            .expect("empty-list fallback creates personal seek point 2222")
            .position,
        crate::ai::Position {
            x: 110.0,
            y: 100.0,
            sector: Some(seek_sector),
            level: 0,
        },
        "personal point 2222 is exactly the actor-centered SeekArea fallback"
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.owner == Some(seeking_filter_zero)
                    && element.command == crate::element::Command::Move
            }),
        "the already-reached personal point must not queue a causal Move"
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
    // The orderless actor reports the no-animation sentinel, so ReturnToDuty's
    // GoTo takes the already-at-post gate (REACHPOINT) before the map-bounds
    // check can fail; the resulting already-facing turn completes immediately
    // and Think(EVENT_DONE) synchronously reaches the virtual enemy SetState
    // tail. Original notifies FilterAIEvent(AISTATE_DEFAULT + 100) after that
    // final state change, so the state callback is the last visible marker.
    assert_eq!(
        &npc_custom_values(&engine, default)[0..3],
        &[101, 101, 1],
        "Think(EVENT_DONE) closes ReturnToDuty and the final Default state callback commits"
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

    // The causal SeekArea Move is only registered at launch; the manager's
    // Hourglass instructs it later in the frame, so the adjacent
    // GetCurrentAction reads the orderless no-animation sentinel.
    assert_eq!(
        npc_custom_values(&engine, actor)[3],
        crate::order::OrderType::NonanimationEnd as i32,
        "the causal SeekArea Move stays registered-not-instructed at the adjacent instruction"
    );
    assert!(
        engine.orders.pending_move_requests.is_empty(),
        "StopAll/Halt cancels the older pre-sequence Move intent before SeekArea queues its causal Move"
    );
    // A registered-but-unlaunched Move that Halt cancels keeps its element in
    // the sequence in a cancelled state; only the to-go registration is
    // removed. The stale 333/444 intent may therefore survive as a dead
    // element, but must never remain runnable.
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .filter(|element| {
                element.owner == Some(actor)
                    && matches!(
                        &element.data,
                        crate::sequence::SequenceElementData::Movement { destination, .. }
                            if *destination == crate::coordinates::MapPoint::new(333.0, 444.0)
                    )
            })
            .all(|element| {
                matches!(
                    element.state,
                    crate::sequence::SequenceState::Impossible
                        | crate::sequence::SequenceState::Interrupted
                )
            }),
        "the stale 333/444 intent must be cancelled at Halt, never runnable as a causal Move"
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
    let door_actor = install_unrelated_multi_exit_building_actor(&mut seeking_engine);
    select_unrelated_pass_door_fixture(&mut seeking_engine, door_actor);
    {
        let entity = seeking_engine.world.entities.get_mut(seeking).unwrap();
        entity
            .element_data_mut()
            .set_position(WorldPoint3D::new(198.0, 100.0, 0.0));
        entity.actor_data_mut().unwrap().old_action = crate::order::OrderType::WaitingUpright;
    }
    // Scratch construction prepares forecasts without drawing; only an AI
    // statement that resolves the door actor's alternatives would draw. The
    // control proves the fixture really carries a resolvable multi-exit gate.
    let control_scratch = seeking_engine.build_sim_scratch(&sim, &seeking_assets);
    let (_, control_trace) = with_draw_trace(|| {
        control_scratch
            .ai_entity_views
            .get(&door_actor.index())
            .expect("unrelated door actor has an AI entity view")
            .forecasted_destination
            .resolve(&sim);
    });
    drop(control_scratch);
    assert!(
        control_trace.contains(&RngSite::BuildingExitGate),
        "the unrelated multi-exit fixture must draw BuildingExitGate when its forecast is resolved"
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
    let fleeing_door_actor = install_unrelated_multi_exit_building_actor(&mut fleeing_engine);
    select_unrelated_pass_door_fixture(&mut fleeing_engine, fleeing_door_actor);
    let (_, fleeing_trace) = with_draw_trace(|| {
        run_ai_state_native_probe(&mut fleeing_engine, &fleeing_assets, fleeing);
    });
    assert!(
        !fleeing_trace.contains(&RngSite::BuildingExitGate),
        "Fleeing/Panic must not forecast an unrelated building actor"
    );
}

#[test]
fn fused_owner_walk_does_not_forecast_rng_for_unrelated_actors() {
    use crate::sim_rng::{RngSite, with_draw_trace};

    let (mut engine, assets, owner) = setup_ai_state_native_probe("EnvelopeRngProbe", 3);
    let door_actor = install_unrelated_multi_exit_building_actor(&mut engine);
    select_unrelated_pass_door_fixture(&mut engine, door_actor);
    engine
        .get_entity_mut(owner)
        .expect("forecast control owner exists")
        .element_data_mut()
        .set_position(WorldPoint3D::new(198.0, 100.0, 0.0));
    let sim = crate::sim_rng::test_context();
    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for (id, entity) in engine.world.entities.occupied() {
        positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
    }

    // Scratch construction prepares forecasts without drawing; the control
    // proves the unrelated door actor's alternatives would draw if resolved.
    let control_scratch = engine.build_sim_scratch(&sim, &assets);
    let (_, control_trace) = with_draw_trace(|| {
        control_scratch
            .ai_entity_views
            .get(&door_actor.index())
            .expect("unrelated door actor has an AI entity view")
            .forecasted_destination
            .resolve(&sim);
    });
    drop(control_scratch);
    assert!(
        control_trace.contains(&RngSite::BuildingExitGate),
        "the fixture must prove that resolving the unrelated door actor's forecast would draw"
    );
    let (_, fused_trace) =
        with_draw_trace(|| engine.tick_actor_owner_envelopes(&sim, &assets, &positions));

    assert!(engine.get_entity(owner).is_some());
    assert!(
        !fused_trace.contains(&RngSite::BuildingExitGate),
        "the fused walk may forecast only at a consuming NPC owner, never because an unrelated actor exists: {fused_trace:?}"
    );
}

#[test]
fn unrelated_detection_event_does_not_resolve_entering_primary_or_officer_forecasts() {
    use crate::ai::{AiLockFlags, Stimulus, StimulusType};
    use crate::element::ActiveDoorPass;
    use crate::gate::DoorIndex;
    use crate::profiles::ProfileRank;
    use crate::sim_rng::{RngSite, with_draw_trace};
    use std::collections::VecDeque;

    let (mut engine, assets, owner) = setup_ai_state_native_probe("DetectionRngProbe", 3);
    let entering_primary = install_unrelated_multi_exit_building_actor(&mut engine);
    let entering_officer = engine.add_entity(make_scripted_soldier(""));
    let owner_camp = engine
        .get_entity(owner)
        .expect("detection RNG owner exists")
        .camp();
    let Entity::Soldier(officer) = engine.get_entity_mut(entering_officer).unwrap() else {
        unreachable!()
    };
    officer.element.active = true;
    officer.soldier.cached_camp = owner_camp;
    officer.actor.active_door_pass = Some(ActiveDoorPass {
        door_index: DoorIndex(0),
        direct: true,
        position_direct: true,
        steps: VecDeque::new(),
        triggers_fired: 0,
        current_action: crate::order::OrderType::default(),
        current_reverse: false,
        saved_action_state: None,
    });
    let officer_ai = officer.npc.ai_brain.enemy_mut().unwrap();
    officer_ai.soldier_profile_rank = ProfileRank::Officer;
    officer_ai.hth_weapon_id = 1;

    let owner_ai = engine
        .get_entity_mut(owner)
        .and_then(Entity::enemy_ai_mut)
        .expect("detection RNG owner has Enemy AI");
    owner_ai.base.primary_target = entering_primary.index();
    owner_ai.missed_pc = entering_primary.index();
    owner_ai.base.locks_flag_field = AiLockFlags::FREEZE;
    owner_ai
        .base
        .outbox
        .detection
        .stimuli
        .push(Stimulus::with_human(
            StimulusType::EventView,
            entering_primary.index(),
        ));

    let sim = crate::sim_rng::test_context();
    let (_, trace) = with_draw_trace(|| {
        engine.tick_enemy_ai_drain_pending_stimuli_for_npc(&sim, owner, &assets, None, None)
    });
    assert!(
        !trace.contains(&RngSite::BuildingExitGate),
        "retaining an unrelated EVENT_VIEW may prepare primary/missed/officer alternatives but must not resolve any forecast: {trace:?}"
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
    engine.world.fast_grid_mut().size_map(64, 64);
    engine.world.fast_grid_mut().allocate_layers(1);
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

#[test]
fn initialization_binds_scripts_before_draining_init_one_ai_state_callbacks() {
    let mut engine = EngineInner::new();
    let civilian = engine.add_entity(make_scripted_civilian("InitStateRecorder"));
    engine
        .world
        .entities
        .get_mut(civilian)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .initial_action = crate::order::OrderType::WaitingUpright as u32;
    engine
        .world
        .entities
        .get_mut(civilian)
        .unwrap()
        .npc_data_mut()
        .unwrap()
        .custom_values[9] = 4;

    let mut assets = install_state_change_script(
        &mut engine,
        state_change_scb(vec![state_change_filter_class(
            "InitStateRecorder",
            false,
            None,
        )]),
    );
    engine.initialize(&mut assets);

    let ai = engine
        .world
        .entities
        .get(civilian)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert!(
        ai.outbox.reentrant.owner_work.is_empty(),
        "InitOneAI state work must settle before the first Hourglass tick"
    );
    assert_eq!(
        npc_custom_values(&engine, civilian)[4],
        101,
        "the bound actor VM must receive Default's AI_STATE_CHANGE callback during InitOneAI"
    );
}

#[test]
fn initialization_caches_the_aspect_adjusted_initial_view_direction() {
    let mut engine = EngineInner::new();
    let diagonal = engine.add_entity(make_scripted_civilian(""));
    let cardinal = engine.add_entity(make_scripted_civilian(""));
    engine
        .get_entity_mut(diagonal)
        .expect("diagonal civilian")
        .position_iface_mut()
        .set_direction_instantly(crate::position_interface::Direction::from_raw(2));
    engine
        .get_entity_mut(cardinal)
        .expect("cardinal civilian")
        .position_iface_mut()
        .set_direction_instantly(crate::position_interface::Direction::from_raw(4));

    engine.initialize(&mut LevelAssets::new());

    let initial_view = |actor| {
        engine
            .get_entity(actor)
            .expect("initialized civilian")
            .ai_controller()
            .expect("civilian AI")
            .initial_view_direction
    };
    assert_eq!(
        initial_view(diagonal),
        1,
        "StoreInitialPositionParameters uses aspect 1 for the vector, but FaceTo bins that \
         vector with ASPECT_RATIO"
    );
    assert_eq!(
        initial_view(cardinal),
        4,
        "the aspect conversion must preserve cardinal authored directions"
    );
}

#[test]
fn direct_ai_drain_consumes_civilian_blink_enemy_request() {
    let mut engine = EngineInner::new();
    let civilian = engine.add_entity(make_scripted_civilian(""));
    engine
        .world
        .entities
        .get_mut(civilian)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .outbox
        .actor
        .blink_all_enemies = true;

    engine.drain_direct_ai_owner_boundary(
        &crate::sim_rng::test_context(),
        civilian,
        &LevelAssets::new(),
    );

    assert!(
        !engine
            .world
            .entities
            .get(civilian)
            .unwrap()
            .ai_controller()
            .unwrap()
            .outbox
            .actor
            .blink_all_enemies,
        "RHElementActorNPC::BlinkEnemy(NULL) must settle for civilians as well as soldiers"
    );
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
fn ai_human_handle_resolution_preserves_soldier_kind() {
    let mut engine = EngineInner::new();
    let target = engine.add_entity(make_scripted_soldier("SoldierTarget"));

    assert_eq!(
        engine.expect_human_id_for_ai_handle(target.index(), "test target"),
        target,
        "legacy AI handles must resolve through the occupied entity slot instead of being reconstructed as PCs"
    );
}

#[test]
fn ai_focus_accepts_live_object_element() {
    let mut engine = EngineInner::new();
    let observer = engine.add_entity(make_scripted_soldier(""));
    let ale = engine.add_entity(Entity::Bonus(ElementBonus {
        element: ElementData {
            kind: ElementKind::ObjectBonus,
            active: true,
            ..Default::default()
        },
        object: ObjectData {
            object_type: ObjectType::Ale,
            ..Default::default()
        },
    }));
    engine
        .world
        .entities
        .get_mut(observer)
        .and_then(Entity::ai_controller_mut)
        .expect("observer retains AI")
        .outbox
        .actor
        .set_focus(ale.index());

    engine.drain_pending_for_npc(
        &crate::sim_rng::test_context(),
        observer,
        &LevelAssets::new(),
    );

    let npc = engine
        .world
        .entities
        .get(observer)
        .and_then(Entity::npc_data)
        .expect("observer retains NPC data");
    assert_eq!(npc.follow_target, Some(ale));
    assert_eq!(npc.eye_status, crate::element::EyeStatus::Follow);
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
        6,
        "the observable special-strike substate must emit and drain its Enemy callback"
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

    let mut unscripted_tail = EngineInner::new();
    let actor = unscripted_tail.add_entity(make_scripted_soldier(""));
    queue_seeking(&mut unscripted_tail, actor);
    {
        let ai = unscripted_tail
            .world
            .entities
            .get_mut(actor)
            .unwrap()
            .ai_controller_mut()
            .unwrap();
        ai.set_ai_state(crate::ai::AiState::Default);
        ai.current_substate = crate::ai::Substate::DefaultEnroute;
    }
    unscripted_tail.drain_ai_state_change_notifications_for(&sim, &assets, actor);
    let ai = unscripted_tail
        .world
        .entities
        .get(actor)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert_eq!(ai.current_state, crate::ai::AiState::Default);
    assert_eq!(ai.current_substate, crate::ai::Substate::DefaultEnroute);
    assert!(
        ai.outbox.reentrant.owner_work.is_empty(),
        "an unavailable callback is consumed without rewinding a later handler-tail state"
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

#[test]
fn fit_again_engine_calls_surround_state_callback_in_original_order() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::element::{Detectable, DetectableType, EyeStatus};

    fn observe(friendly: bool) {
        let mut engine = EngineInner::new();
        let class_name = if friendly {
            "FriendlyWakeOrder"
        } else {
            "EnemyWakeOrder"
        };
        let owner = if friendly {
            engine.add_entity(make_scripted_civilian(class_name))
        } else {
            engine.add_entity(make_scripted_soldier(class_name))
        };
        let observer = engine.add_entity(make_scripted_civilian(""));
        let mut assets = install_state_change_script(
            &mut engine,
            state_change_scb(vec![ai_state_native_probe_class(class_name, 1, 1)]),
        );
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles
            .soldiers
            .resize_with(1, crate::profiles::SoldierProfile::default);
        profiles.hth_weapons.resize_with(1, Default::default);
        profiles.soldiers[0].hth_weapon_id = 1;
        bind_state_change_actor(&mut engine, owner, class_name);
        if let Some(enemy) = engine
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::enemy_ai_mut)
        {
            enemy.hth_weapon_id = 1;
        }

        let npc = engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .npc_data_mut()
            .unwrap();
        npc.eye_status = EyeStatus::Closed;
        let ai = npc.ai_brain.base_mut().unwrap();
        ai.current_state = AiState::Sleeping;
        ai.current_substate = Substate::SleepingUnconscious;
        ai.timer_is_running = false;
        ai.outbox
            .detection
            .stimuli
            .push(Stimulus::new(StimulusType::EventFitAgain));
        engine
            .world
            .entities
            .get_mut(observer)
            .unwrap()
            .npc_data_mut()
            .unwrap()
            .detectable_lists[DetectableType::Body as usize]
            .push(Detectable {
                element: Some(owner),
                detectable_type: DetectableType::Body,
                ..Detectable::default()
            });

        let (woke, observations) = super::script::capture_ai_state_callback_observations(|| {
            crate::sim_rng::with_seed(0xA013_F17A, |sim| {
                engine.dispatch_pending_fit_again_for_npc(sim, owner, &assets)
            })
        });
        assert!(woke);
        let observation = observations
            .iter()
            .find(|observation| observation.owner == owner)
            .unwrap_or_else(|| panic!("{class_name} emitted no state callback observation"));
        assert_eq!(
            observation.body_references_to_owner, 0,
            "InformEveryoneOnMyResurrection must finish before SetState"
        );
        if friendly {
            assert_eq!(
                observation.eye_status,
                EyeStatus::LookForward,
                "friendly SetViewStatus precedes ReturnToDuty/SetState"
            );
        } else {
            assert_eq!(
                observation.eye_status,
                EyeStatus::Closed,
                "enemy SetViewStatus follows SetState"
            );
            assert!(
                !observation.timer_is_running,
                "enemy LaunchTimer follows SetState"
            );
        }
        let owner_npc = engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .npc_data()
            .unwrap();
        assert_eq!(owner_npc.eye_status, EyeStatus::LookForward);
        if !friendly {
            assert!(owner_npc.ai_brain.base().unwrap().timer_is_running);
        }
    }

    observe(false);
    observe(true);
}
