//! Native bindings registered onto a [`MissionLuaState`].
//!
//! Every Spellforge `api.lua` entry that mission scripts actually
//! call gets a Rust shim here. The shim runs against the engine's
//! `GameHost` (the same dispatcher the `.scb` VM uses), so a Lua
//! script and an `.scb` script behave identically when they invoke
//! the same engine function.
//!
//! ## Host pointer plumbing
//!
//! `mlua` requires registered functions to be `'static`, but the
//! `GameHost` we mutate lives on the engine and has a lifetime tied
//! to the current event call. We stash a raw pointer to it in Lua's
//! app-data ([`HostPtr`]) for the duration of one event:
//!
//! ```ignore
//! state.lua().set_app_data(HostPtr::new(host));
//! state.run("Initialize")?;
//! state.lua().remove_app_data::<HostPtr>();
//! ```
//!
//! The safety contract is **scoped access**: callers may only invoke
//! Lua entry points wrapped in [`MissionLuaState::with_host`] (added
//! by the event-dispatch layer in `engine/script.rs`), which
//! guarantees the pointer is live and exclusively borrowed.
//!
//! ## Alias table
//!
//! Several Spellforge names are 1:1 renames of engine natives we
//! already implement (`SequenceMove` → `RecordMove`, `AssignPatrol`
//! → `AssignPath`, …). Rather than duplicate the dispatch arms we
//! just register the same Rust shim under both names — see
//! [`NATIVE_ALIASES`].

use std::cell::Cell;

use mlua::{Function, Lua, Table, Value};
use robin_engine::interp::NativeStack;
use robin_engine::natives::{GameHost, NATIVE_REGISTRY, NativeFn};

use crate::state::MissionLuaState;

/// Type-erased pointer to the engine's [`GameHost`] for the
/// duration of one Lua event invocation. See module docs for the
/// safety contract.
///
/// Stored as Lua app data; closures retrieve it via
/// [`Lua::app_data_ref`].
#[derive(Clone)]
pub(crate) struct HostPtr(Cell<*mut GameHost>);

// SAFETY: `HostPtr` is only accessed from the thread that called
// [`MissionLuaState::with_host`]; we never let it escape Lua, and
// Lua itself is `Send` (mlua's `send` feature). Sync is not needed.
unsafe impl Send for HostPtr {}

impl HostPtr {
    pub(crate) fn new(host: *mut GameHost) -> Self {
        Self(Cell::new(host))
    }

    /// Borrow the host mutably. Panics if the pointer is null,
    /// which means a script reached a native outside of a
    /// [`MissionLuaState::with_host`] scope (a host bug, not a
    /// script bug).
    ///
    /// Returns a raw pointer rather than `&mut GameHost` so the
    /// Rust borrow checker doesn't infer a conflicting lifetime
    /// between repeated calls within the same native — every
    /// shim re-derefs at the top so the lifetime is fresh per
    /// call. Clippy's `mut_from_ref` lint correctly flags the
    /// alternative `&self -> &mut T` shape as a lifetime lie.
    fn host_ptr(&self) -> *mut GameHost {
        let ptr = self.0.get();
        assert!(
            !ptr.is_null(),
            "robin_lua: native invoked with no GameHost attached; \
             wrap the call site in MissionLuaState::with_host"
        );
        ptr
    }
}

/// Spellforge name → engine name aliases.
///
/// The Spellforge `api.lua` invented several "friendlier" names for
/// natives that already exist in Robin's `.scb` VM. We map both
/// names to the same dispatch arm so mission scripts written
/// against either spelling work.
///
/// **Sequence* / Record***: Spellforge surfaces sequence-recording
/// natives under `Sequence<Verb>` (e.g. `SequenceMove`) while
/// Robin's `.scb` calls them `Record<Verb>` (e.g. `RecordMove`).
/// Same dispatch; the Lua side just uses the friendlier name.
pub const NATIVE_ALIASES: &[(&str, NativeFn)] = &[
    // Sequence brackets
    ("StartSequence", NativeFn::Start),
    ("EndSequence", NativeFn::Thanx),
    // Record* → Sequence* (the names Spellforge missions use)
    ("SequenceScrollCameraTo", NativeFn::RecordScrollCameraTo),
    ("SequenceJumpCameraTo", NativeFn::RecordJumpCameraTo),
    ("SequenceSetZoomLevel", NativeFn::RecordSetZoom),
    ("SequenceMoveCameraTo", NativeFn::RecordMoveCameraTo),
    ("SequenceDisplayMap", NativeFn::RecordDisplayMap),
    ("SequenceMove", NativeFn::RecordMove),
    ("SequenceMoveIntoBuilding", NativeFn::RecordMoveIntoBuilding),
    ("SequenceMoveNear", NativeFn::RecordMoveNear),
    ("SequenceEnterLevel", NativeFn::RecordEnterGame),
    ("SequenceLeaveLevel", NativeFn::RecordLeaveGame),
    ("SequenceTurnTo", NativeFn::RecordTurnTo),
    ("SequencePlayAnim", NativeFn::RecordPlayAnim),
    ("SequencePlayAnimLoop", NativeFn::RecordPlayAnimLoop),
    ("SequencePlayAnimFreeze", NativeFn::RecordPlayAnimFreeze),
    ("SequencePlayDialog", NativeFn::RecordPlayDialog),
    ("SequenceReplaceAnim", NativeFn::RecordReplaceAnim),
    ("SequenceRestoreAnim", NativeFn::RecordRestoreAnim),
    ("SequenceLockAI", NativeFn::RecordLockAI),
    ("SequenceUnlockAI", NativeFn::RecordUnlockAI),
    ("SequenceLockUser", NativeFn::RecordLockUser),
    ("SequenceUnLockUser", NativeFn::RecordUnLockUser),
    ("SequenceLockCameraOn", NativeFn::RecordLockCameraOn),
    ("SequenceClearCameraLock", NativeFn::RecordClearCameraLock),
    ("SequenceTimer", NativeFn::RecordTimer),
    ("SequenceSpeak", NativeFn::RecordSpeak),
    ("SequenceSpeakPC", NativeFn::RecordSpeakPC),
    ("SequenceFreezeAll", NativeFn::RecordFreezeAll),
    ("SequenceDisplayPopupText", NativeFn::RecordDisplayPopupText),
    ("SequenceSendMessage", NativeFn::RecordSendMessage),
    (
        "SequenceSendMessageWithArguments",
        NativeFn::RecordSendMessageWithArguments,
    ),
    ("SequenceSeekActor", NativeFn::RecordSeekActor),
    ("SequenceSeekActorMessage", NativeFn::RecordSeekActorMessage),
    (
        "SequenceSeekActorMessageWithArguments",
        NativeFn::RecordSeekActorMessageWithArguments,
    ),
    (
        "SequenceActivateMobileElement",
        NativeFn::RecordActivateMobileElement,
    ),
    (
        "SequenceDeactivateMobileElement",
        NativeFn::RecordDeactivateMobileElement,
    ),
    (
        "SequenceStartMobileElement",
        NativeFn::RecordStartMobileElement,
    ),
    (
        "SequenceStopMobileElement",
        NativeFn::RecordStopMobileElement,
    ),
    ("SequenceTakeCorpse", NativeFn::RecordTakeCorpse),
    ("SequenceLeaveCorpse", NativeFn::RecordLeaveCorpse),
    ("SequenceAction", NativeFn::RecordAction),
    ("SequenceActionAvailable", NativeFn::RecordActionAvailable),
    (
        "SequenceCharacterAvailable",
        NativeFn::RecordCharacterAvailable,
    ),
    ("SequenceUnBlip", NativeFn::RecordUnBlip),
    // Non-Sequence name renames
    ("AssignPatrol", NativeFn::AssignPath),
    ("AddAsSquadMember", NativeFn::AddAsSubordinate),
    ("RemoveAllSquadMembers", NativeFn::RemoveAllSubordinates),
    ("GetScrollState", NativeFn::GetScrollStatus),
    ("SetScrollState", NativeFn::SetScrollStatus),
    (
        "AreAllEnemiesInsideOutOfAction",
        NativeFn::AreAllEnemiesInsideHS,
    ),
];

/// Descriptor for one Lua-side binding. The dispatcher used by all
/// "calls a NativeFn" shims pushes the args onto a NativeStack in
/// the order the engine expects, invokes `GameHost::call(index)`,
/// then returns the result.
pub struct NativeBinding {
    pub lua_name: &'static str,
    pub native: NativeFn,
}

/// Register every binding (engine-backed natives, Lua-only
/// helpers, and aliases) onto `state`. Idempotent: subsequent calls
/// are no-ops with a warning.
pub fn register_natives(state: &mut MissionLuaState) -> mlua::Result<()> {
    if state.natives_registered() {
        tracing::warn!("register_natives called twice on the same MissionLuaState");
        return Ok(());
    }
    let lua = state.lua();
    let globals = lua.globals();

    // 1. Every NativeFn the .scb VM knows about is registered under
    //    its canonical name. Argument coercion is "all ints", which
    //    matches the engine's stack-based dispatcher (every native
    //    pops i32s and returns an i32). String-taking and
    //    table-returning natives are handled in Lua-only shims
    //    below — they bypass NativeStack.
    for definition in NATIVE_REGISTRY
        .iter()
        .filter(|definition| definition.expose_to_lua)
    {
        let f = make_native_shim(lua, definition.native)?;
        globals.set(definition.signature.name, f)?;
    }

    // 2. Aliases — Spellforge names that map onto an existing
    //    NativeFn. Same shim, different global name.
    for (alias, target) in NATIVE_ALIASES {
        let f = make_native_shim(lua, *target)?;
        globals.set(*alias, f)?;
    }

    // 3. Lua-only natives that don't go through NativeStack.
    register_lua_only(lua, &globals)?;

    // 4. Freeze the global environment. After `sandbox(true)`,
    //    script code can still read `GetActor` etc. but writes
    //    are diverted into a per-script environment table — so a
    //    misbehaving mission can't, say, replace `GetGlobal` with
    //    a fake version that lies to the next mission. This is
    //    the reason this crate chose Luau over Lua 5.4. See the
    //    crate-level docs for the full security story.
    lua.sandbox(true)?;

    state.mark_natives_registered();
    Ok(())
}

/// Build a Lua function that pushes its integer args onto a
/// `NativeStack` (in argument order) and calls `GameHost::call`.
fn make_native_shim(lua: &Lua, native: NativeFn) -> mlua::Result<Function> {
    let sig = robin_engine::natives::native_signature_by_name(native.into())
        .expect("every NativeFn has a signature");
    let arity = sig.params.len();
    let index = native as u32;
    lua.create_function(move |lua, args: mlua::Variadic<Value>| {
        if args.len() != arity {
            return Err(mlua::Error::RuntimeError(format!(
                "{}: expected {} arg(s), got {}",
                sig.name,
                arity,
                args.len()
            )));
        }
        let host_ptr = lua
            .app_data_ref::<HostPtr>()
            .ok_or_else(|| {
                mlua::Error::RuntimeError(format!("{}: called with no GameHost attached", sig.name))
            })?
            .clone();
        // SAFETY: see HostPtr module docs — the pointer is
        // exclusively borrowed for the duration of `with_host`,
        // which is the only place this shim runs.
        let host: &mut GameHost = unsafe { &mut *host_ptr.host_ptr() };
        let mut stack = NativeStack::default();
        // Push in argument order — the engine's `pop_i32()` pulls
        // them off in *reverse*, so the last arg ends up on top of
        // the stack. The .scb VM produced this exact order, so we
        // mirror it.
        for v in args.iter() {
            stack.push_i32(value_to_i32(v, sig.name)?);
        }
        let ret = <GameHost as robin_engine::interp::HostFunctions>::call(host, index, &mut stack);
        Ok(ret)
    })
}

/// Coerce a Lua value into an i32 the engine expects. Booleans map
/// to 0/1 (Spellforge's `IsActorPC` etc. return booleans on the Lua
/// side but bytes on the engine side). Floats are accepted because
/// some natives take `bits_of(f32)` packed as i32 (zoom, weights).
fn value_to_i32(v: &Value, native: &str) -> mlua::Result<i32> {
    match v {
        Value::Integer(i) => Ok(*i),
        Value::Number(n) => {
            // Whole numbers pass straight through; non-whole values
            // are packed as f32 bits (the engine pops them with
            // `f32::from_bits`). This matches what the .scb compiler
            // emits for literal floats.
            if n.fract() == 0.0 && *n >= i32::MIN as f64 && *n <= i32::MAX as f64 {
                Ok(*n as i32)
            } else {
                Ok((*n as f32).to_bits() as i32)
            }
        }
        Value::Boolean(b) => Ok(i32::from(*b)),
        Value::Nil => Ok(0),
        other => Err(mlua::Error::RuntimeError(format!(
            "{native}: cannot coerce {} to int",
            other.type_name()
        ))),
    }
}

fn register_lua_only(lua: &Lua, globals: &Table) -> mlua::Result<()> {
    // ── Name-table lookups (return entity handle / null) ──
    //
    // Spellforge's `.rhm` extension prefixes each entity with a
    // string identifier. The mission loader fills the matching
    // BTreeMap on GameHost; these natives just look up by name.
    let get_actor = lua.create_function(|lua, name: String| {
        // SAFETY: see HostPtr docs — pointer is valid for the
        // duration of the surrounding `with_host` scope.
        let host: &mut GameHost = unsafe { &mut *host_ptr(lua)? };
        Ok(host.lua_actor_names.get(&name).copied().unwrap_or(0))
    })?;
    globals.set("GetActor", get_actor)?;

    let get_item = lua.create_function(|lua, name: String| {
        // SAFETY: see HostPtr docs — pointer is valid for the
        // duration of the surrounding `with_host` scope.
        let host: &mut GameHost = unsafe { &mut *host_ptr(lua)? };
        Ok(host.lua_item_names.get(&name).copied().unwrap_or(0))
    })?;
    globals.set("GetItem", get_item)?;

    let get_location = lua.create_function(|lua, name: String| {
        // SAFETY: see HostPtr docs — pointer is valid for the
        // duration of the surrounding `with_host` scope.
        let host: &mut GameHost = unsafe { &mut *host_ptr(lua)? };
        Ok(host.lua_location_names.get(&name).copied().unwrap_or(0))
    })?;
    globals.set("GetLocation", get_location)?;

    let get_patrol = lua.create_function(|lua, name: String| {
        // SAFETY: see HostPtr docs — pointer is valid for the
        // duration of the surrounding `with_host` scope.
        let host: &mut GameHost = unsafe { &mut *host_ptr(lua)? };
        Ok(host.lua_patrol_names.get(&name).copied().unwrap_or(0))
    })?;
    globals.set("GetPatrol", get_patrol)?;

    let get_scroll = lua.create_function(|lua, name: String| {
        // SAFETY: see HostPtr docs — pointer is valid for the
        // duration of the surrounding `with_host` scope.
        let host: &mut GameHost = unsafe { &mut *host_ptr(lua)? };
        Ok(host.lua_scroll_names.get(&name).copied().unwrap_or(0))
    })?;
    globals.set("GetScroll", get_scroll)?;

    // ── Reverse lookup: handle → name ──
    let get_actor_name = lua.create_function(|lua, handle: i32| {
        // SAFETY: see HostPtr docs — pointer is valid for the
        // duration of the surrounding `with_host` scope.
        let host: &mut GameHost = unsafe { &mut *host_ptr(lua)? };
        // Linear scan — Spellforge's DLL does the same. The maps
        // are mission-scoped (low hundreds of entries), so this
        // doesn't merit a reverse index.
        for (name, h) in &host.lua_actor_names {
            if *h == handle {
                return Ok(name.clone());
            }
        }
        // Spellforge returns the literal "<not found>" sentinel
        // when no name matches — preserved here for script parity.
        Ok("<not found>".to_owned())
    })?;
    globals.set("GetActorName", get_actor_name)?;

    // ── Whole-table dumps ──
    //
    // Used by Spellforge's `lib/common.lua` to iterate every named
    // actor and assign patrols / cutscene roles in bulk.
    let get_all_actors = lua.create_function(|lua, ()| {
        // SAFETY: see HostPtr docs — pointer is valid for the
        // duration of the surrounding `with_host` scope.
        let host: &mut GameHost = unsafe { &mut *host_ptr(lua)? };
        let t = lua.create_table_with_capacity(0, host.lua_actor_names.len())?;
        for (name, handle) in &host.lua_actor_names {
            t.set(name.clone(), *handle)?;
        }
        Ok(t)
    })?;
    globals.set("GetAllActors", get_all_actors)?;

    // ── Sequence callbacks ──
    //
    // `SequenceCall(fn)` stashes the Lua closure in
    // `SequenceCallbacks[next_id]`, then queues an engine message
    // with that id. When the engine later dispatches that message,
    // the host's event router pulls the closure back out and runs
    // it. Counter starts at 10_000 to avoid colliding with
    // engine-defined message ids (which all sit below).
    //
    // The id-counter is stored on the SequenceCallbacks table
    // itself (`__next_id` key) so it survives Lua's GC and stays
    // mission-scoped without needing a Rust-side counter.
    let sequence_call = lua.create_function(|lua, callback: Function| {
        // The callback table lives in the Lua registry, not in
        // `_G`, so the sandbox's frozen-globals rule doesn't
        // block writes. See `state::SEQUENCE_CALLBACKS_KEY`.
        let callbacks: Table = lua.named_registry_value(crate::state::SEQUENCE_CALLBACKS_KEY)?;
        let next: i32 = callbacks.get("__next_id").unwrap_or(10_000);
        callbacks.set(next, callback)?;
        callbacks.set("__next_id", next + 1)?;

        // Tell the engine to send message `next` to God (null
        // actor handle) — when it dispatches we pull the closure
        // back out. Equivalent to Spellforge's
        // `SequenceSendMessage(God(), id)`.
        // SAFETY: see HostPtr docs — pointer is valid for the
        // duration of the surrounding `with_host` scope.
        let host: &mut GameHost = unsafe { &mut *host_ptr(lua)? };
        let mut stack = NativeStack::default();
        // RecordSendMessage(actor, message) pops `message` first
        // (top of stack), then `actor`. So push actor, then
        // message — matching the engine's evaluation order.
        stack.push_i32(0); // actor = God
        stack.push_i32(next);
        <GameHost as robin_engine::interp::HostFunctions>::call(
            host,
            NativeFn::RecordSendMessage as u32,
            &mut stack,
        );
        Ok(())
    })?;
    globals.set("SequenceCall", sequence_call)?;

    Ok(())
}

/// Helper for Lua-only shims that need the host. Wraps the app-data
/// lookup with a clearer error message than the raw assertion in
/// `HostPtr::host_ptr`.
///
/// Returns a `*mut` rather than `&mut` so the borrow-checker
/// doesn't infer a conflicting lifetime between repeated calls
/// inside the same shim — each call site dereferences afresh.
fn host_ptr(lua: &Lua) -> mlua::Result<*mut GameHost> {
    let ptr = lua.app_data_ref::<HostPtr>().ok_or_else(|| {
        mlua::Error::RuntimeError("robin_lua: native invoked with no GameHost attached".to_owned())
    })?;
    Ok(ptr.host_ptr())
}

// Canonical Lua enumeration is declared by
// `robin_engine::natives::NATIVE_REGISTRY`; see `register_natives`.
