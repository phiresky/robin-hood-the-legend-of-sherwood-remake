//! End-to-end smoke test: spin up a `MissionLuaState`, attach a
//! `GameHost`, run a Lua snippet that calls registered natives, and
//! confirm the side-effects landed on the host.

use mlua::Lua;
use robin_engine::entities::Entities;
use robin_engine::natives::{
    EngineCommand, GameHost, NativeSessionCapabilities, ObjectiveChange, ScriptHandleCodec,
    ScriptState,
};
use robin_lua::{MissionLuaState, NativeAbiError, register_natives};

fn fresh_state() -> (MissionLuaState, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut state = MissionLuaState::new(dir.path()).expect("new");
    register_natives(&mut state).expect("register");
    (state, dir)
}

fn test_soldier() -> robin_engine::element::Entity {
    robin_engine::element::Entity::Soldier(robin_engine::element::ActorSoldier {
        element: robin_engine::element::ElementData {
            kind: robin_engine::element::ElementKind::ActorSoldier,
            ..Default::default()
        },
        actor: robin_engine::element::ActorData::default(),
        human: robin_engine::element::HumanData::default(),
        npc: robin_engine::element::NpcData {
            ai_brain: robin_engine::element::AiBrain::Enemy(Box::new(
                robin_engine::ai_enemy::EnemyAi::new(0),
            )),
            ..Default::default()
        },
        soldier: robin_engine::element::SoldierData::default(),
    })
}

#[test]
fn lua_natives_mutate_canonical_entity_ai_and_grid_owners() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::from_legacy_slots(vec![Some(test_soldier())]);
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    std::sync::Arc::make_mut(&mut fast_grid.level).sectors.push(
        robin_engine::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: robin_engine::coordinates::MapBBox::default(),
            sector_type: robin_engine::sector::SectorType::DOOR,
            layer: 0,
            sector_number: robin_engine::sector::SectorNumber::default(),
            door_index: Some(0),
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
        },
    );
    fast_grid.sector_active.push(true);
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut domains_value =
        serde_json::to_value(robin_engine::engine::ScriptDomains::default()).unwrap();
    domains_value["interactables"]["doors"] =
        serde_json::to_value(vec![robin_engine::gate::Door::default()]).unwrap();
    let mut script_domains = serde_json::from_value(domains_value).unwrap();
    let bindings = robin_engine::natives::AttachedScriptBindings {
        script_location_count: 1,
        script_point_count: 1,
        location_positions: std::sync::Arc::new(vec![(12.0, 34.0)]),
        location_layers: std::sync::Arc::new(vec![2]),
        location_sectors: std::sync::Arc::new(vec![3]),
        ..Default::default()
    };
    let mut script_state = ScriptState::default();
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let point = ScriptHandleCodec::location_handle_from_index(0);
    let door = ScriptHandleCodec::door_handle_from_index(0);

    state
        .with_host_state_and_bindings(
            &mut host,
            &mut script_state,
            &mut script_domains,
            &bindings,
            &capabilities,
            |lua: &Lua| {
                let value: i32 = lua
                    .load(format!(
                        "SetCustomNPCValue({actor}, 3, 77); AddRepulsivePoint({point}, 5, 4, 3); ActivateDoorMouseSector(false, {door}); return GetCustomNPCValue({actor}, 3)"
                    ))
                    .eval()?;
                assert_eq!(value, 77);
                Ok(())
            },
        )
        .unwrap();

    drop(capabilities);
    assert_eq!(
        entities
            .get_legacy_slot(0)
            .expect("soldier remains canonical")
            .1
            .npc_data()
            .expect("entity remains an NPC")
            .custom_values[3],
        77
    );
    assert_eq!(ai_global.repulsive_points.len(), 1);
    assert!(!fast_grid.is_sector_active(0));
}

/// `InitGlobal(0, 42)` from Lua must land in `GameHost::globals`.
#[test]
fn engine_native_called_from_lua_writes_host_state() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let mut script_state = ScriptState::default();
    state
        .with_host_and_state(
            &mut host,
            &mut script_state,
            &mut script_domains,
            &capabilities,
            |lua: &Lua| lua.load("InitGlobal(0, 42)").exec(),
        )
        .unwrap();
    assert_eq!(script_state.globals.get(&0).copied(), Some(42));
}

/// `Start()` from Lua must open a `RecordingSession`. Confirms the
/// dispatcher maps declared bool returns to Lua booleans.
#[test]
fn start_then_thanx_round_trips() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    state
        .with_host(
            &mut host,
            &mut script_domains,
            &capabilities,
            |lua: &Lua| {
                let start_ret: bool = lua.load("return Start()").eval()?;
                assert!(start_ret);
                // `Thanx` on an empty recording returns 0 with a
                // warning — engine semantics preserved.
                let thanx_ret: bool = lua.load("return Thanx()").eval()?;
                assert!(!thanx_ret);
                Ok(())
            },
        )
        .unwrap();
}

/// `StartSequence` is the Spellforge alias for `Start`. After the
/// call the engine must have an active recording — confirms both
/// the alias and the host-pointer plumbing.
#[test]
fn spellforge_alias_opens_recording() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let mut script_state = ScriptState::default();
    state
        .with_host_and_state(
            &mut host,
            &mut script_state,
            &mut script_domains,
            &capabilities,
            |lua: &Lua| lua.load("StartSequence()").exec(),
        )
        .unwrap();
    assert!(
        script_state.sequence_recorder.recording.is_some(),
        "StartSequence should have opened a recording"
    );
}

/// `GetActor("Robin")` returns the registered handle when the
/// mission loader has populated `lua_actor_names`, else 0.
#[test]
fn get_actor_name_lookup() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let mut bindings = robin_engine::natives::AttachedScriptBindings::default();
    std::sync::Arc::make_mut(&mut bindings.lua_names)
        .actors
        .insert("RobinHood".to_owned(), 7);
    let mut script_state = robin_engine::natives::ScriptState::default();

    state
        .with_host_state_and_bindings(
            &mut host,
            &mut script_state,
            &mut script_domains,
            &bindings,
            &capabilities,
            |lua: &Lua| {
                let hit: i32 = lua.load("return GetActor('RobinHood')").eval()?;
                assert_eq!(hit, 7);
                let miss: i32 = lua.load("return GetActor('Nobody')").eval()?;
                assert_eq!(miss, 0);
                let name: String = lua.load("return GetActorName(7)").eval()?;
                assert_eq!(name, "RobinHood");
                let unknown: String = lua.load("return GetActorName(999)").eval()?;
                assert_eq!(unknown, "<not found>");
                Ok(())
            },
        )
        .unwrap();
}

/// `GetAllActors()` returns a name→handle table — `lib/common.lua`
/// relies on this for bulk patrol assignment.
#[test]
fn get_all_actors_dumps_table() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let mut bindings = robin_engine::natives::AttachedScriptBindings::default();
    let names = std::sync::Arc::make_mut(&mut bindings.lua_names);
    names.actors.insert("Alice".to_owned(), 1);
    names.actors.insert("Bob".to_owned(), 2);
    let mut script_state = robin_engine::natives::ScriptState::default();

    state
        .with_host_state_and_bindings(
            &mut host,
            &mut script_state,
            &mut script_domains,
            &bindings,
            &capabilities,
            |lua: &Lua| {
                let alice: i32 = lua.load("return GetAllActors().Alice").eval()?;
                let bob: i32 = lua.load("return GetAllActors().Bob").eval()?;
                assert_eq!(alice, 1);
                assert_eq!(bob, 2);
                Ok(())
            },
        )
        .unwrap();
}

/// `AddObjective(7, true)` and `CompleteObjective(7)` queue an
/// `ObjectiveChange` for the host to drain — these are the
/// Spellforge-only natives we added in this PR.
#[test]
fn add_and_complete_objective_queue_changes() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    state
        .with_host(
            &mut host,
            &mut script_domains,
            &capabilities,
            |lua: &Lua| {
                lua.load("AddObjective(7, true); CompleteObjective(7)")
                    .exec()
            },
        )
        .unwrap();
    assert_eq!(host.pending_objective_changes.len(), 2);
    assert!(matches!(
        host.pending_objective_changes[0],
        ObjectiveChange::Add {
            id: 7,
            is_main: true
        }
    ));
    assert!(matches!(
        host.pending_objective_changes[1],
        ObjectiveChange::Complete { id: 7 }
    ));
}

/// `IsActorOutOfAction` is the Spellforge English-name alias for
/// `IsActorHS`. With no entities set up, the native warns and
/// returns 0; we just confirm it's reachable through the binding.
#[test]
fn is_actor_out_of_action_callable() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    state
        .with_host(
            &mut host,
            &mut script_domains,
            &capabilities,
            |lua: &Lua| {
                let r: bool = lua.load("return IsActorOutOfAction(99)").eval()?;
                assert!(!r);
                Ok(())
            },
        )
        .unwrap();
}

/// The table is deliberately heterogeneous: it proves the original
/// `RHScriptAPI.scs` signature, rather than the Lua value's apparent shape,
/// selects each stack encoding and return conversion.
#[test]
fn native_abi_is_signature_driven() {
    let cases = [
        (
            // Luau stores this literal as an integer. SetZoomLevel declares a
            // float, so the ABI must still pack the bits for 1.0f32.
            "return SetZoomLevel(1)",
            "boolean",
            Some(EngineCommand::SetZoomLevel { zoom: 1.0 }),
        ),
        (
            "return DisplayMap(true)",
            "boolean",
            Some(EngineCommand::DisplayMap { show: true }),
        ),
        (
            "return DisplayMap(false)",
            "boolean",
            Some(EngineCommand::DisplayMap { show: false }),
        ),
        ("return InitGlobal(7.0, 9.0)", "nil", None),
    ];

    for (source, expected_return_type, expected_command) in cases {
        let (state, _dir) = fresh_state();
        let mut host = GameHost::new();
        let mut entities = Entities::new();
        let mut ai_global = robin_engine::ai::AiGlobalState::default();
        let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
        let capabilities =
            NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
        let mut script_domains = robin_engine::engine::ScriptDomains::default();
        let mut script_state = ScriptState::default();
        state
            .with_host_and_state(
                &mut host,
                &mut script_state,
                &mut script_domains,
                &capabilities,
                |lua: &Lua| {
                    let return_type: String = lua
                        .load(format!("return type((function() {source} end)())"))
                        .eval()?;
                    assert_eq!(return_type, expected_return_type, "{source}");
                    Ok(())
                },
            )
            .unwrap();

        match expected_command {
            Some(EngineCommand::SetZoomLevel { zoom }) => assert!(matches!(
                host.commands.as_slice(),
                [EngineCommand::SetZoomLevel { zoom: actual }] if *actual == zoom
            )),
            Some(EngineCommand::DisplayMap { show }) => assert!(matches!(
                host.commands.as_slice(),
                [EngineCommand::DisplayMap { show: actual }] if *actual == show
            )),
            None => {
                assert!(host.commands.is_empty());
                assert_eq!(script_state.globals.get(&7), Some(&9));
            }
            Some(other) => panic!("test case does not handle command {other:?}"),
        }
    }
}

/// Invalid values must remain typed Rust errors through mlua's callback
/// wrapper; they must never be reinterpreted as zero-valued stack words.
#[test]
fn invalid_native_arguments_are_typed_errors() {
    let cases = [
        ("InitGlobal(1, 2.5)", "integral 32-bit value"),
        ("InitGlobal(1, true)", "expects integer"),
        ("SetZoomLevel(true)", "expects number"),
        ("DisplayMap(1)", "expects boolean"),
        ("DisplayMap(nil)", "expects boolean"),
        ("DisplayMap()", "expected 1 argument(s)"),
    ];

    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    state
        .with_host(
            &mut host,
            &mut script_domains,
            &capabilities,
            |lua: &Lua| {
                for (source, expected_message) in cases {
                    let err = lua.load(source).exec().expect_err(source);
                    assert!(
                        err.to_string().contains(expected_message),
                        "{source}: unexpected error: {err}"
                    );
                    assert!(
                        contains_native_abi_error(&err),
                        "{source}: NativeAbiError was erased: {err}"
                    );
                }
                Ok(())
            },
        )
        .unwrap();
}

fn contains_native_abi_error(error: &mlua::Error) -> bool {
    match error {
        mlua::Error::CallbackError { cause, .. } => contains_native_abi_error(cause),
        mlua::Error::ExternalError(cause) => cause.downcast_ref::<NativeAbiError>().is_some(),
        _ => false,
    }
}

/// `SequenceCall(fn)` registers a Lua closure in the registry-side
/// callback stash and queues a sequence-recorded SendMessage with
/// the matching id. Confirms the callback indexing (starts at
/// 10_000 to avoid colliding with engine-defined message ids).
#[test]
fn sequence_call_registers_callback() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    state
        .with_host(
            &mut host,
            &mut script_domains,
            &capabilities,
            |lua: &Lua| {
                // A SequenceCall must happen inside a recording — open
                // one first so the engine doesn't reject the queued
                // SendMessage.
                lua.load("StartSequence(); SequenceCall(function() return 1 end)")
                    .exec()?;
                // Counter advanced past 10_000 → exactly one callback
                // was registered. Read through the registry directly —
                // the table is intentionally hidden from `_G`.
                let callbacks: mlua::Table =
                    lua.named_registry_value("robin_lua.sequence_callbacks")?;
                let stash_id: i32 = callbacks.get("__next_id")?;
                assert_eq!(stash_id, 10_001);
                let kind = callbacks
                    .get::<mlua::Value>(10_000_i32)?
                    .type_name()
                    .to_string();
                assert_eq!(kind, "function");
                Ok(())
            },
        )
        .unwrap();
}

/// Natives must error cleanly if invoked outside `with_host`.
#[test]
fn no_host_attached_errors() {
    let (state, _dir) = fresh_state();
    let err = state.lua().load("InitGlobal(0, 42)").exec().unwrap_err();
    assert!(
        err.to_string().contains("no GameHost attached"),
        "unexpected error: {err}"
    );
}

/// `with_host` must clear the native session when the closure exits,
/// otherwise a follow-up call (without a fresh `with_host` scope)
/// would silently read a freed pointer.
#[test]
fn native_session_cleared_after_scope() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    state
        .with_host(
            &mut host,
            &mut script_domains,
            &capabilities,
            |_lua: &Lua| Ok(()),
        )
        .unwrap();
    let err = state.lua().load("InitGlobal(0, 1)").exec().unwrap_err();
    assert!(err.to_string().contains("no GameHost attached"));
}

/// Returning an error must detach the host just like a successful return.
#[test]
fn native_session_cleared_after_error() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut script_domains = robin_engine::engine::ScriptDomains::default();

    let result: mlua::Result<()> = state.with_host(
        &mut host,
        &mut script_domains,
        &capabilities,
        |_lua: &Lua| Err(mlua::Error::RuntimeError("deliberate error".into())),
    );
    assert!(result.is_err());

    let err = state.lua().load("InitGlobal(0, 1)").exec().unwrap_err();
    assert!(err.to_string().contains("no GameHost attached"));
}

/// Reject a nested attachment before it can replace the outer session. After
/// the typed error, the outer attachment must remain usable.
#[test]
fn nested_session_rejection_preserves_outer_session() {
    let (state, _dir) = fresh_state();
    let mut outer_host = GameHost::new();
    let mut outer_entities = Entities::new();
    let mut outer_ai = robin_engine::ai::AiGlobalState::default();
    let mut outer_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let outer_capabilities =
        NativeSessionCapabilities::new(&mut outer_entities, &mut outer_ai, &mut outer_grid);
    let mut outer_domains = robin_engine::engine::ScriptDomains::default();
    let mut outer_script_state = ScriptState::default();
    let mut nested_host = GameHost::new();
    let mut nested_entities = Entities::new();
    let mut nested_ai = robin_engine::ai::AiGlobalState::default();
    let mut nested_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let nested_capabilities =
        NativeSessionCapabilities::new(&mut nested_entities, &mut nested_ai, &mut nested_grid);
    let mut nested_domains = robin_engine::engine::ScriptDomains::default();

    state
        .with_host_and_state(
            &mut outer_host,
            &mut outer_script_state,
            &mut outer_domains,
            &outer_capabilities,
            |lua: &Lua| -> mlua::Result<()> {
                let nested = state.with_host(
                    &mut nested_host,
                    &mut nested_domains,
                    &nested_capabilities,
                    |_lua: &Lua| Ok(()),
                );
                let error = nested.expect_err("nested session must be rejected");
                assert!(
                    error
                        .to_string()
                        .contains("nested Lua native-call sessions")
                );
                lua.load("InitGlobal(9, 81)").exec()?;
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(outer_script_state.globals.get(&9), Some(&81));
}

#[test]
fn synchronous_nested_lua_calls_share_one_native_session() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let mut script_state = ScriptState::default();

    state
        .with_host_and_state(
            &mut host,
            &mut script_state,
            &mut script_domains,
            &capabilities,
            |lua: &Lua| {
                lua.load(
                    r#"
                    local function descend(depth)
                        if depth == 0 then
                            InitGlobal(3, 1)
                            SetGlobal(3, 42)
                            return GetGlobal(3)
                        end
                        return descend(depth - 1)
                    end
                    assert(descend(8) == 42)
                    "#,
                )
                .exec()
            },
        )
        .unwrap();

    assert_eq!(script_state.globals.get(&3), Some(&42));
}

#[test]
fn rust_to_lua_reentrancy_reuses_the_active_session() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let mut script_state = ScriptState::default();

    state
        .with_host_and_state(
            &mut host,
            &mut script_state,
            &mut script_domains,
            &capabilities,
            |lua: &Lua| {
                lua.load("InitGlobal(4, 10)").exec()?;
                let reenter = lua
                    .create_function(|lua, ()| lua.load("SetGlobal(4, GetGlobal(4) + 5)").exec())?;
                reenter.call::<()>(())?;
                Ok(())
            },
        )
        .unwrap();

    assert_eq!(script_state.globals.get(&4), Some(&15));
}

#[test]
fn retained_function_cannot_reuse_a_stale_session() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let mut script_state = ScriptState::default();

    let stale_function: mlua::Function = state
        .with_host_and_state(
            &mut host,
            &mut script_state,
            &mut script_domains,
            &capabilities,
            |lua: &Lua| {
                lua.load("InitGlobal(5, 1)").exec()?;
                lua.load("return function() SetGlobal(5, 99) end").eval()
            },
        )
        .unwrap();

    let error = stale_function.call::<()>(()).unwrap_err();
    assert!(
        error.to_string().contains("no GameHost attached"),
        "unexpected stale-function error: {error}"
    );
    assert_eq!(script_state.globals.get(&5), Some(&1));
}

#[test]
#[should_panic(expected = "deliberate native-session unwind")]
fn panic_unwind_detaches_native_session() {
    struct VerifyDetachedOnUnwind<'state>(&'state MissionLuaState);

    impl Drop for VerifyDetachedOnUnwind<'_> {
        fn drop(&mut self) {
            let error = self
                .0
                .lua()
                .load("InitGlobal(0, 1)")
                .exec()
                .expect_err("native session survived panic unwind");
            assert!(error.to_string().contains("no GameHost attached"));
        }
    }

    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let _verify_detached = VerifyDetachedOnUnwind(&state);

    let _: mlua::Result<()> = state.with_host(
        &mut host,
        &mut script_domains,
        &capabilities,
        |_lua: &Lua| panic!("deliberate native-session unwind"),
    );
}

#[test]
fn native_dispatch_preserves_game_host_queue_order() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut entities = Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let capabilities =
        NativeSessionCapabilities::new(&mut entities, &mut ai_global, &mut fast_grid);
    let mut script_domains = robin_engine::engine::ScriptDomains::default();

    state
        .with_host(
            &mut host,
            &mut script_domains,
            &capabilities,
            |lua: &Lua| {
                lua.load(
                    "DisplayMap(true); SetZoomLevel(2); DisplayMap(false); \
                     AddObjective(10, true); CompleteObjective(10); AddObjective(11, false)",
                )
                .exec()
            },
        )
        .unwrap();

    assert!(matches!(
        host.commands.as_slice(),
        [
            EngineCommand::DisplayMap { show: true },
            EngineCommand::SetZoomLevel { zoom },
            EngineCommand::DisplayMap { show: false },
        ] if *zoom == 2.0
    ));
    assert!(matches!(
        host.pending_objective_changes.as_slice(),
        [
            ObjectiveChange::Add {
                id: 10,
                is_main: true,
            },
            ObjectiveChange::Complete { id: 10 },
            ObjectiveChange::Add {
                id: 11,
                is_main: false,
            },
        ]
    ));
}
