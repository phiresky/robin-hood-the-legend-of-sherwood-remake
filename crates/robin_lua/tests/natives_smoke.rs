//! End-to-end smoke test: spin up a `MissionLuaState`, attach a
//! `GameHost`, run a Lua snippet that calls registered natives, and
//! confirm the side-effects landed on the host.

use mlua::Lua;
use robin_engine::natives::{GameHost, ObjectiveChange};
use robin_lua::{MissionLuaState, register_natives};

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
    state
        .with_host(&mut host, |lua: &Lua| lua.load("InitGlobal(0, 42)").exec())
        .unwrap();
    assert_eq!(host.globals.get(&0).copied(), Some(42));
}

/// `Start()` from Lua must open a `RecordingSession`. Confirms the
/// dispatcher routes integer args + return values cleanly.
#[test]
fn start_then_thanx_round_trips() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    state
        .with_host(&mut host, |lua: &Lua| {
            let start_ret: i32 = lua.load("return Start()").eval()?;
            assert_eq!(start_ret, 1);
            // `Thanx` on an empty recording returns 0 with a
            // warning — engine semantics preserved.
            let thanx_ret: i32 = lua.load("return Thanx()").eval()?;
            assert_eq!(thanx_ret, 0);
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
    state
        .with_host(&mut host, |lua: &Lua| lua.load("StartSequence()").exec())
        .unwrap();
    assert!(
        host.recording.is_some(),
        "StartSequence should have opened a recording"
    );
}

/// `GetActor("Robin")` returns the registered handle when the
/// mission loader has populated `lua_actor_names`, else 0.
#[test]
fn get_actor_name_lookup() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    host.lua_actor_names.insert("RobinHood".to_owned(), 7);

    state
        .with_host(&mut host, |lua: &Lua| {
            let hit: i32 = lua.load("return GetActor('RobinHood')").eval()?;
            assert_eq!(hit, 7);
            let miss: i32 = lua.load("return GetActor('Nobody')").eval()?;
            assert_eq!(miss, 0);
            let name: String = lua.load("return GetActorName(7)").eval()?;
            assert_eq!(name, "RobinHood");
            let unknown: String = lua.load("return GetActorName(999)").eval()?;
            assert_eq!(unknown, "<not found>");
            Ok(())
        })
        .unwrap();
}

/// `GetAllActors()` returns a name→handle table — `lib/common.lua`
/// relies on this for bulk patrol assignment.
#[test]
fn get_all_actors_dumps_table() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    host.lua_actor_names.insert("Alice".to_owned(), 1);
    host.lua_actor_names.insert("Bob".to_owned(), 2);

    state
        .with_host(&mut host, |lua: &Lua| {
            let alice: i32 = lua.load("return GetAllActors().Alice").eval()?;
            let bob: i32 = lua.load("return GetAllActors().Bob").eval()?;
            assert_eq!(alice, 1);
            assert_eq!(bob, 2);
            Ok(())
        })
        .unwrap();
}

/// `AddObjective(7, true)` and `CompleteObjective(7)` queue an
/// `ObjectiveChange` for the host to drain — these are the
/// Spellforge-only natives we added in this PR.
#[test]
fn add_and_complete_objective_queue_changes() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    state
        .with_host(&mut host, |lua: &Lua| {
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
    state
        .with_host(&mut host, |lua: &Lua| {
            let r: i32 = lua.load("return IsActorOutOfAction(99)").eval()?;
            assert_eq!(r, 0);
            Ok(())
        })
        .unwrap();
}

/// `SequenceCall(fn)` registers a Lua closure in the registry-side
/// callback stash and queues a sequence-recorded SendMessage with
/// the matching id. Confirms the callback indexing (starts at
/// 10_000 to avoid colliding with engine-defined message ids).
#[test]
fn sequence_call_registers_callback() {
    let (state, _dir) = fresh_state();
    let mut host = GameHost::new();
    state
        .with_host(&mut host, |lua: &Lua| {
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
    state.with_host(&mut host, |_lua: &Lua| Ok(())).unwrap();
    let err = state.lua().load("InitGlobal(0, 1)").exec().unwrap_err();
    assert!(err.to_string().contains("no GameHost attached"));
}
