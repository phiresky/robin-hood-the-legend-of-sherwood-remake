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
//! `GameHost` and [`NativeSessionCapabilities`] live on the engine and have
//! lifetimes tied to the current event call. Lua app-data stores ordinary raw
//! pointers plus exactly one lifetime-erased capability-bundle pointer in
//! [`HostPtr`] for the duration of one event:
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
//! guarantees every pointer is live and exclusively borrowed. The
//! [`crate::state::HostAttachment`] guard removes the app-data on both success
//! and error; registered shims recover the capability reference only for the
//! synchronous native call and never let it escape to Lua or an upvalue.
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
use robin_engine::engine::ScriptDomains;
use robin_engine::interp::{NativeCallOutcome, NativeStack};
use robin_engine::natives::{
    AttachedScriptBindings, GameHost, NATIVE_REGISTRY, NativeContext, NativeFn,
    NativeSessionCapabilities, NativeSignature, ScriptState,
};

use crate::state::MissionLuaState;

/// Type-erased pointers attached for one Lua event invocation. The single
/// `capabilities` pointer represents entities, AI, grid, campaign/stat, and
/// immutable query views together; adding parallel erased owner pointers
/// would break the one-session-bundle invariant. See module docs for the
/// safety contract.
///
/// Stored as Lua app data; closures retrieve it via
/// [`Lua::app_data_ref`].
#[derive(Clone)]
pub(crate) struct HostPtr {
    host: Cell<*mut GameHost>,
    script_state: Cell<*mut ScriptState>,
    script_domains: Cell<*mut ScriptDomains>,
    bindings: Cell<*const AttachedScriptBindings>,
    capabilities: Cell<*const ()>,
}

// SAFETY: `HostPtr` is only accessed from the thread that called
// [`MissionLuaState::with_host`]; we never let it escape Lua, and
// Lua itself is `Send` (mlua's `send` feature). Sync is not needed.
unsafe impl Send for HostPtr {}

impl HostPtr {
    pub(crate) fn new(
        host: *mut GameHost,
        script_state: *mut ScriptState,
        script_domains: *mut ScriptDomains,
        bindings: *const AttachedScriptBindings,
        capabilities: &NativeSessionCapabilities<'_>,
    ) -> Self {
        // SAFETY CONTRACT: the reference lifetime is erased only because
        // HostAttachment removes this HostPtr before the enclosing
        // with_host_state_and_bindings call returns. Native shims reborrow it
        // synchronously and NativeContext retains only short RefMut guards and
        // copied immutable query references, never this bundle reference.
        Self {
            host: Cell::new(host),
            script_state: Cell::new(script_state),
            script_domains: Cell::new(script_domains),
            bindings: Cell::new(bindings),
            capabilities: Cell::new(capabilities as *const _ as *const ()),
        }
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
        let ptr = self.host.get();
        assert!(
            !ptr.is_null(),
            "robin_lua: native invoked with no GameHost attached; \
             wrap the call site in MissionLuaState::with_host"
        );
        ptr
    }

    fn script_state_ptr(&self) -> *mut ScriptState {
        let ptr = self.script_state.get();
        assert!(
            !ptr.is_null(),
            "robin_lua: native invoked with no ScriptState attached; wrap the call site in MissionLuaState::with_host"
        );
        ptr
    }

    fn capabilities_ptr(&self) -> *const () {
        let ptr = self.capabilities.get();
        assert!(
            !ptr.is_null(),
            "robin_lua: native invoked with no session capabilities attached"
        );
        ptr
    }

    fn script_domains_ptr(&self) -> *mut ScriptDomains {
        let ptr = self.script_domains.get();
        assert!(
            !ptr.is_null(),
            "robin_lua: native invoked with no ScriptDomains capability attached; wrap the call site in MissionLuaState::with_host"
        );
        ptr
    }

    fn bindings_ptr(&self) -> *const AttachedScriptBindings {
        let ptr = self.bindings.get();
        assert!(
            !ptr.is_null(),
            "robin_lua: native invoked with no ScriptBindings attached"
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

/// A typed failure at the Lua/native ABI boundary.
///
/// These errors are wrapped in [`mlua::Error::ExternalError`], preserving
/// their concrete type for callers that need to distinguish bad script
/// arguments from failures raised inside a native implementation.
#[derive(Debug, thiserror::Error)]
pub enum NativeAbiError {
    #[error("native `{native}` has no signature metadata")]
    MissingSignature { native: &'static str },
    #[error("native `{native}` has unsupported {position} type `{declared_type}`")]
    UnsupportedSignatureType {
        native: &'static str,
        position: &'static str,
        declared_type: &'static str,
    },
    #[error("{native}: expected {expected} argument(s), got {actual}")]
    WrongArity {
        native: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("{native}: argument {index} (`{parameter}`) expects {expected}, got Lua {actual}")]
    WrongArgumentType {
        native: &'static str,
        index: usize,
        parameter: &'static str,
        expected: &'static str,
        actual: &'static str,
    },
    #[error(
        "{native}: argument {index} (`{parameter}`) must be an integral 32-bit value, got {value}"
    )]
    InvalidInteger {
        native: &'static str,
        index: usize,
        parameter: &'static str,
        value: f64,
    },
    #[error(
        "{native}: argument {index} (`{parameter}`) must be representable as a finite f32, got {value}"
    )]
    InvalidFloat {
        native: &'static str,
        index: usize,
        parameter: &'static str,
        value: f64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeAbiType {
    Int,
    Float,
    Bool,
    Handle,
    Void,
}

impl NativeAbiType {
    fn from_signature(
        native: &'static str,
        position: &'static str,
        declared_type: &'static str,
    ) -> Result<Self, NativeAbiError> {
        match declared_type {
            "int" => Ok(Self::Int),
            "float" => Ok(Self::Float),
            "bool" => Ok(Self::Bool),
            "Actor" | "Door" | "Patch" | "Location" | "SoundSource" | "Building" | "Scroll"
            | "Way" => Ok(Self::Handle),
            "void" if position == "return" => Ok(Self::Void),
            _ => Err(NativeAbiError::UnsupportedSignatureType {
                native,
                position,
                declared_type,
            }),
        }
    }

    fn lua_name(self) -> &'static str {
        match self {
            Self::Int => "integer",
            Self::Float => "number",
            Self::Bool => "boolean",
            Self::Handle => "handle",
            Self::Void => "no value",
        }
    }
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

    // 1. Every NativeFn exposed to Lua by the declarative registry is
    //    registered under its canonical name. The signature controls how
    //    Lua values are encoded into the engine's four-byte stack words and
    //    how its i32 result word is exposed to Lua. String-taking and
    //    table-returning natives are handled in Lua-only shims below — they
    //    bypass NativeStack.
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

/// Build a Lua function that marshals its declared arguments onto a
/// `NativeStack` (in argument order), calls `GameHost::call`, and marshals
/// the declared return type back to Lua.
///
/// Original provenance: `original-code/RHScriptAPI.scs` is the source of
/// the `int`/`float`/`bool`/`void`/handle signatures mirrored by
/// [`NativeSignature`]. In particular, floats always occupy the stack as
/// IEEE-754 `f32` bits even when their Lua value is mathematically integral.
///
/// TODO(parity): The Spellforge add-on DLL that bridged Lua to this API is
/// not present in `original-code`; verify whether it accepted additional
/// cross-type coercions. Until then the declared signature is enforced so a
/// bad value cannot silently turn into an unrelated stack word.
fn make_native_shim(lua: &Lua, native: NativeFn) -> mlua::Result<Function> {
    let sig = robin_engine::natives::native_signature_by_name(native.into()).ok_or_else(|| {
        mlua::Error::external(NativeAbiError::MissingSignature {
            native: native.into(),
        })
    })?;
    let param_types = sig
        .params
        .iter()
        .map(|param| NativeAbiType::from_signature(sig.name, "parameter", param.ty))
        .collect::<Result<Vec<_>, _>>()
        .map_err(mlua::Error::external)?;
    let return_type = NativeAbiType::from_signature(sig.name, "return", sig.return_type)
        .map_err(mlua::Error::external)?;
    let arity = sig.params.len();
    let index = native as u32;
    lua.create_function(move |lua, args: mlua::Variadic<Value>| {
        if args.len() != arity {
            return Err(mlua::Error::external(NativeAbiError::WrongArity {
                native: sig.name,
                expected: arity,
                actual: args.len(),
            }));
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
        let capabilities: &NativeSessionCapabilities<'_> = unsafe {
            &*(host_ptr.capabilities_ptr() as *const NativeSessionCapabilities<'_>)
        };
        let script_state: &mut ScriptState = unsafe { &mut *host_ptr.script_state_ptr() };
        let script_domains: &mut ScriptDomains = unsafe { &mut *host_ptr.script_domains_ptr() };
        let bindings: &AttachedScriptBindings = unsafe { &*host_ptr.bindings_ptr() };
        let mut native_context = NativeContext::with_bindings(
            host,
            script_state,
            script_domains,
            bindings,
            capabilities,
        );
        let mut stack = NativeStack::default();
        // Push in argument order — the engine's `pop_i32()` pulls
        // them off in *reverse*, so the last arg ends up on top of
        // the stack. The .scb VM produced this exact order, so we
        // mirror it.
        for (index, ((value, param), abi_type)) in args
            .iter()
            .zip(sig.params.iter())
            .zip(param_types.iter())
            .enumerate()
        {
            stack.push_i32(argument_to_stack_word(
                value, *abi_type, sig, index, param.name,
            )?);
        }
        match robin_engine::interp::HostFunctions::call(&mut native_context, index, &mut stack) {
            NativeCallOutcome::Return(ret) => Ok(return_from_stack_word(ret, return_type)),
            NativeCallOutcome::PendingNestedCall(call) => Err(mlua::Error::RuntimeError(format!(
                "{} requires nested script dispatch, which is unavailable through the Lua host adapter: {call:?}",
                sig.name
            ))),
        }
    })
}

fn argument_to_stack_word(
    value: &Value,
    abi_type: NativeAbiType,
    signature: &'static NativeSignature,
    index: usize,
    parameter: &'static str,
) -> mlua::Result<i32> {
    let wrong_type = || {
        mlua::Error::external(NativeAbiError::WrongArgumentType {
            native: signature.name,
            index: index + 1,
            parameter,
            expected: abi_type.lua_name(),
            actual: value.type_name(),
        })
    };

    match abi_type {
        NativeAbiType::Int | NativeAbiType::Handle => match value {
            Value::Integer(value) => i32::try_from(*value).map_err(|_| {
                mlua::Error::external(NativeAbiError::InvalidInteger {
                    native: signature.name,
                    index: index + 1,
                    parameter,
                    value: *value as f64,
                })
            }),
            Value::Number(value)
                if value.is_finite()
                    && value.fract() == 0.0
                    && *value >= i32::MIN as f64
                    && *value <= i32::MAX as f64 =>
            {
                Ok(*value as i32)
            }
            Value::Number(value) => Err(mlua::Error::external(NativeAbiError::InvalidInteger {
                native: signature.name,
                index: index + 1,
                parameter,
                value: *value,
            })),
            _ => Err(wrong_type()),
        },
        NativeAbiType::Float => {
            let value = match value {
                Value::Integer(value) => *value as f64,
                Value::Number(value) => *value,
                _ => return Err(wrong_type()),
            };
            let packed = value as f32;
            if !value.is_finite() || !packed.is_finite() {
                return Err(mlua::Error::external(NativeAbiError::InvalidFloat {
                    native: signature.name,
                    index: index + 1,
                    parameter,
                    value,
                }));
            }
            Ok(packed.to_bits() as i32)
        }
        NativeAbiType::Bool => match value {
            Value::Boolean(value) => Ok(i32::from(*value)),
            _ => Err(wrong_type()),
        },
        NativeAbiType::Void => Err(mlua::Error::external(
            NativeAbiError::UnsupportedSignatureType {
                native: signature.name,
                position: "parameter",
                declared_type: "void",
            },
        )),
    }
}

fn return_from_stack_word(value: i32, abi_type: NativeAbiType) -> Value {
    match abi_type {
        NativeAbiType::Int | NativeAbiType::Handle => Value::Integer(value.into()),
        NativeAbiType::Float => Value::Number(f32::from_bits(value as u32) as f64),
        NativeAbiType::Bool => Value::Boolean(value != 0),
        NativeAbiType::Void => Value::Nil,
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
        let names = unsafe { &*bindings_ptr(lua)? };
        Ok(names.lua_names.actors.get(&name).copied().unwrap_or(0))
    })?;
    globals.set("GetActor", get_actor)?;

    let get_item = lua.create_function(|lua, name: String| {
        // SAFETY: see HostPtr docs — pointer is valid for the
        // duration of the surrounding `with_host` scope.
        let names = unsafe { &*bindings_ptr(lua)? };
        Ok(names.lua_names.items.get(&name).copied().unwrap_or(0))
    })?;
    globals.set("GetItem", get_item)?;

    let get_location = lua.create_function(|lua, name: String| {
        // SAFETY: see HostPtr docs — pointer is valid for the
        // duration of the surrounding `with_host` scope.
        let names = unsafe { &*bindings_ptr(lua)? };
        Ok(names.lua_names.locations.get(&name).copied().unwrap_or(0))
    })?;
    globals.set("GetLocation", get_location)?;

    let get_patrol = lua.create_function(|lua, name: String| {
        // SAFETY: see HostPtr docs — pointer is valid for the
        // duration of the surrounding `with_host` scope.
        let names = unsafe { &*bindings_ptr(lua)? };
        Ok(names.lua_names.patrols.get(&name).copied().unwrap_or(0))
    })?;
    globals.set("GetPatrol", get_patrol)?;

    let get_scroll = lua.create_function(|lua, name: String| {
        // SAFETY: see HostPtr docs — pointer is valid for the
        // duration of the surrounding `with_host` scope.
        let names = unsafe { &*bindings_ptr(lua)? };
        Ok(names.lua_names.scrolls.get(&name).copied().unwrap_or(0))
    })?;
    globals.set("GetScroll", get_scroll)?;

    // ── Reverse lookup: handle → name ──
    let get_actor_name = lua.create_function(|lua, handle: i32| {
        // SAFETY: see HostPtr docs — pointer is valid for the
        // duration of the surrounding `with_host` scope.
        let names = unsafe { &*bindings_ptr(lua)? };
        // Linear scan — Spellforge's DLL does the same. The maps
        // are mission-scoped (low hundreds of entries), so this
        // doesn't merit a reverse index.
        for (name, h) in &names.lua_names.actors {
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
        let names = unsafe { &*bindings_ptr(lua)? };
        let t = lua.create_table_with_capacity(0, names.lua_names.actors.len())?;
        for (name, handle) in &names.lua_names.actors {
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
        let capabilities: &NativeSessionCapabilities<'_> =
            unsafe { &*(capabilities_ptr(lua)? as *const NativeSessionCapabilities<'_>) };
        let script_state: &mut ScriptState = unsafe { &mut *script_state_ptr(lua)? };
        let script_domains: &mut ScriptDomains = unsafe { &mut *script_domains_ptr(lua)? };
        let bindings: &AttachedScriptBindings = unsafe { &*bindings_ptr(lua)? };
        let mut native_context = NativeContext::with_bindings(
            host,
            script_state,
            script_domains,
            bindings,
            capabilities,
        );
        let mut stack = NativeStack::default();
        // RecordSendMessage(actor, message) pops `message` first
        // (top of stack), then `actor`. So push actor, then
        // message — matching the engine's evaluation order.
        stack.push_i32(0); // actor = God
        stack.push_i32(next);
        robin_engine::interp::HostFunctions::call(
            &mut native_context,
            NativeFn::RecordSendMessage as u32,
            &mut stack,
        )
        .expect_return("Lua SequenceCall/RecordSendMessage");
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

fn capabilities_ptr(lua: &Lua) -> mlua::Result<*const ()> {
    let ptr = lua.app_data_ref::<HostPtr>().ok_or_else(|| {
        mlua::Error::RuntimeError(
            "robin_lua: native invoked with no session capabilities attached".to_owned(),
        )
    })?;
    Ok(ptr.capabilities_ptr())
}

fn script_state_ptr(lua: &Lua) -> mlua::Result<*mut ScriptState> {
    let ptr = lua.app_data_ref::<HostPtr>().ok_or_else(|| {
        mlua::Error::RuntimeError(
            "robin_lua: native invoked with no ScriptState attached".to_owned(),
        )
    })?;
    Ok(ptr.script_state_ptr())
}

fn script_domains_ptr(lua: &Lua) -> mlua::Result<*mut ScriptDomains> {
    let ptr = lua.app_data_ref::<HostPtr>().ok_or_else(|| {
        mlua::Error::RuntimeError("native called with no ScriptDomains attached".into())
    })?;
    Ok(ptr.script_domains_ptr())
}

fn bindings_ptr(lua: &Lua) -> mlua::Result<*const AttachedScriptBindings> {
    let ptr = lua.app_data_ref::<HostPtr>().ok_or_else(|| {
        mlua::Error::RuntimeError(
            "robin_lua: native invoked with no ScriptBindings attached".to_owned(),
        )
    })?;
    Ok(ptr.bindings_ptr())
}

// Canonical Lua enumeration is declared by
// `robin_engine::natives::NATIVE_REGISTRY`; see `register_natives`.
