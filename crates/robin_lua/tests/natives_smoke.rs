//! End-to-end smoke test: spin up a `MissionLuaState`, attach a
//! `GameHost`, run a Lua snippet that calls registered natives, and
//! confirm the side-effects landed on the host.

use mlua::Lua;
use robin_engine::natives::{EngineCommand, GameHost, ObjectiveChange, ScriptState};
use robin_lua::{MissionLuaState, NativeAbiError, register_natives};

fn fresh_state() -> (MissionLuaState, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut state = MissionLuaState::new(dir.path()).expect("new");
    register_natives(&mut state).expect("register");
    (state, dir)
}

/// `InitGlobal(0, 42)` from Lua must land in `GameHost::globals`.
#[test]
fn engine_native_called_from_lua_writes_host_state() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let mut script_state = ScriptState::default();
    state
        .with_host_and_state(
            &mut host,
            &mut script_state,
            &mut script_domains,
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
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    state
        .with_host(&mut host, &mut script_domains, |lua: &Lua| {
            let start_ret: bool = lua.load("return Start()").eval()?;
            assert!(start_ret);
            // `Thanx` on an empty recording returns 0 with a
            // warning — engine semantics preserved.
            let thanx_ret: bool = lua.load("return Thanx()").eval()?;
            assert!(!thanx_ret);
            Ok(())
        })
        .unwrap();
}

/// `StartSequence` is the Spellforge alias for `Start`. After the
/// call the engine must have an active recording — confirms both
/// the alias and the host-pointer plumbing.
#[test]
fn spellforge_alias_opens_recording() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let mut script_state = ScriptState::default();
    state
        .with_host_and_state(
            &mut host,
            &mut script_state,
            &mut script_domains,
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
            robin_engine::natives::NativeQueryViews::default(),
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
            robin_engine::natives::NativeQueryViews::default(),
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
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    state
        .with_host(&mut host, &mut script_domains, |lua: &Lua| {
            lua.load("AddObjective(7, true); CompleteObjective(7)")
                .exec()
        })
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
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    state
        .with_host(&mut host, &mut script_domains, |lua: &Lua| {
            let r: bool = lua.load("return IsActorOutOfAction(99)").eval()?;
            assert!(!r);
            Ok(())
        })
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
        let mut script_domains = robin_engine::engine::ScriptDomains::default();
        let mut script_state = ScriptState::default();
        state
            .with_host_and_state(
                &mut host,
                &mut script_state,
                &mut script_domains,
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
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    state
        .with_host(&mut host, &mut script_domains, |lua: &Lua| {
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
        })
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
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    state
        .with_host(&mut host, &mut script_domains, |lua: &Lua| {
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
        })
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

/// `with_host` must clear the host pointer when the closure exits,
/// otherwise a follow-up call (without a fresh `with_host` scope)
/// would silently read a freed pointer.
#[test]
fn host_pointer_cleared_after_scope() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    state
        .with_host(&mut host, &mut script_domains, |_lua: &Lua| Ok(()))
        .unwrap();
    let err = state.lua().load("InitGlobal(0, 1)").exec().unwrap_err();
    assert!(err.to_string().contains("no GameHost attached"));
}

/// Returning an error must detach the host just like a successful return.
#[test]
fn host_pointer_cleared_after_error() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    let mut script_domains = robin_engine::engine::ScriptDomains::default();

    let result: mlua::Result<()> = state.with_host(&mut host, &mut script_domains, |_lua: &Lua| {
        Err(mlua::Error::RuntimeError("deliberate error".into()))
    });
    assert!(result.is_err());

    let err = state.lua().load("InitGlobal(0, 1)").exec().unwrap_err();
    assert!(err.to_string().contains("no GameHost attached"));
}

/// Reject a nested attachment before it can replace the outer scope's raw
/// pointers. The workspace uses panic=abort for ordinary binaries, while the
/// Rust test harness still recognizes an expected panic.
#[test]
#[should_panic(expected = "nested Lua host attachments are not supported")]
fn nested_host_attachment_is_rejected_before_replacement() {
    let (state, _dir) = fresh_state();
    let mut outer_host = GameHost::new();
    let mut outer_domains = robin_engine::engine::ScriptDomains::default();
    let mut nested_host = GameHost::new();
    let mut nested_domains = robin_engine::engine::ScriptDomains::default();

    let _ = state.with_host(
        &mut outer_host,
        &mut outer_domains,
        |_lua: &Lua| -> mlua::Result<()> {
            state.with_host(&mut nested_host, &mut nested_domains, |_lua: &Lua| Ok(()))?;
            Ok(())
        },
    );
}
