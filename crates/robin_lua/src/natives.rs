//! Native bindings registered onto a [`MissionLuaState`].
//!
//! Every Spellforge `api.lua` entry that mission scripts actually
//! call gets a Rust shim here. The shim runs against the engine's
//! `GameHost` (the same dispatcher the `.scb` VM uses), so a Lua
//! script and an `.scb` script behave identically when they invoke
//! the same engine function.
//!
//! ## Native-call session plumbing
//!
//! `mlua` requires registered functions to be `'static`, but the engine
//! owners are borrowed only for the current event. A stack-owned
//! [`NativeCallSession`] aggregates those borrows, and Lua app data stores
//! one lifetime-erased pointer handle to that session for the duration of the
//! event. The handle records the attaching thread and serializes mutable
//! session access; it is `Send` through safe standard-library types rather than
//! a manual unsafe implementation.
//!
//! The safety contract is scoped access: callers invoke Lua entry points only
//! through [`MissionLuaState::with_host`]. The
//! [`crate::state::NativeCallAttachment`] guard removes app data on success,
//! error, and unwind. Each shim reborrows the aggregate for one synchronous
//! native call and never exposes it to Lua values or Rust upvalues.
//!
//! `mlua::Scope` is intentionally not used here: scoped callbacks become
//! invalid when their scope exits, while mission globals must retain stable
//! native functions across every event in the mission.
//!
//! ## Alias table
//!
//! Several Spellforge names are 1:1 renames of engine natives we
//! already implement (`SequenceMove` → `RecordMove`, `AssignPatrol`
//! → `AssignPath`, …). Rather than duplicate the dispatch arms we
//! just register the same Rust shim under both names — see
//! [`NATIVE_ALIASES`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::thread::ThreadId;

use mlua::{Function, Lua, Table, Value};
use robin_engine::engine::ScriptDomains;
use robin_engine::interp::{NativeCallOutcome, NativeStack};
use robin_engine::natives::{
    AttachedScriptBindings, GameHost, NATIVE_REGISTRY, NativeContext, NativeFn,
    NativeSessionCapabilities, NativeSignature, ScriptState,
};

use crate::state::MissionLuaState;

/// All engine borrows available to one synchronous Lua event invocation.
///
/// This value stays on the Rust stack. Only [`AttachedNativeCall`]'s guarded
/// pointer handle crosses mlua's `'static` app-data boundary.
pub(crate) struct NativeCallSession<'call, 'owners> {
    host: &'call mut GameHost,
    script_state: &'call mut ScriptState,
    script_domains: &'call mut ScriptDomains,
    bindings: &'call AttachedScriptBindings,
    capabilities: &'call NativeSessionCapabilities<'owners>,
}

impl<'call, 'owners> NativeCallSession<'call, 'owners> {
    pub(crate) fn new(
        host: &'call mut GameHost,
        script_state: &'call mut ScriptState,
        script_domains: &'call mut ScriptDomains,
        bindings: &'call AttachedScriptBindings,
        capabilities: &'call NativeSessionCapabilities<'owners>,
    ) -> Self {
        Self {
            host,
            script_state,
            script_domains,
            bindings,
            capabilities,
        }
    }

    fn native_context(&mut self) -> NativeContext<'_, 'owners> {
        NativeContext::with_bindings(
            self.host,
            self.script_state,
            self.script_domains,
            self.bindings,
            self.capabilities,
        )
    }
}

#[derive(Debug)]
struct AttachedNativeCallState {
    session: AtomicPtr<()>,
    origin_thread: ThreadId,
    in_use: AtomicBool,
}

/// The sole lifetime-erased value installed in Lua app data while a native
/// call session is active.
///
/// All fields are safely `Send + Sync`. The raw pointer is never dereferenced
/// until [`AttachedNativeCall::with_session`] verifies thread affinity and
/// obtains exclusive access for one synchronous shim invocation.
#[derive(Clone, Debug)]
pub(crate) struct AttachedNativeCall(Arc<AttachedNativeCallState>);

struct SessionUseGuard<'attachment>(&'attachment AtomicBool);

impl Drop for SessionUseGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl AttachedNativeCall {
    pub(crate) fn new(session: &mut NativeCallSession<'_, '_>) -> Self {
        Self(Arc::new(AttachedNativeCallState {
            session: AtomicPtr::new(std::ptr::from_mut(session).cast()),
            origin_thread: std::thread::current().id(),
            in_use: AtomicBool::new(false),
        }))
    }

    pub(crate) fn is_same_attachment(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub(crate) fn invalidate(&self) {
        self.0
            .session
            .store(std::ptr::null_mut(), Ordering::Release);
    }

    /// Reborrow the stack-owned session for one synchronous shim invocation.
    ///
    /// The attachment scope gate keeps the stack frame alive. Thread affinity
    /// prevents `mlua`'s `send` support from moving borrowed, potentially
    /// non-`Sync` engine owners across threads. The in-use flag prevents two
    /// overlapping mutable reborrows, including callback reentrancy while a
    /// native is still dispatching. The higher-ranked callback prevents its
    /// result from depending on either erased lifetime.
    fn with_session<R>(
        &self,
        f: impl for<'call, 'owners> FnOnce(&mut NativeCallSession<'call, 'owners>) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        if std::thread::current().id() != self.0.origin_thread {
            return Err(mlua::Error::RuntimeError(
                "robin_lua: native-call session cannot be used from a thread other than the attaching thread"
                    .to_owned(),
            ));
        }

        self.0
            .in_use
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                mlua::Error::RuntimeError(
                    "robin_lua: native-call session is already in use by another native".to_owned(),
                )
            })?;
        let _use_guard = SessionUseGuard(&self.0.in_use);

        let session = self
            .0
            .session
            .load(Ordering::Acquire)
            .cast::<NativeCallSession<'static, 'static>>();
        if session.is_null() {
            return Err(mlua::Error::RuntimeError(
                "robin_lua: native-call session is no longer attached".to_owned(),
            ));
        }
        // SAFETY: `NativeCallAttachment` holds the per-state scope gate until
        // it removes and invalidates this handle, so a non-null pointer's
        // stack-owned session outlives this call. The origin-thread check
        // occurs before this dereference, keeping all borrowed capabilities on
        // their creating thread. `_use_guard` makes this the sole mutable
        // reborrow and clears that claim on return, error, or unwind. The HRTB
        // prevents `R` from carrying erased borrows.
        unsafe { f(&mut *session) }
    }
}

fn with_attached_session<R>(
    lua: &Lua,
    missing: impl FnOnce() -> mlua::Error,
    f: impl for<'call, 'owners> FnOnce(&mut NativeCallSession<'call, 'owners>) -> mlua::Result<R>,
) -> mlua::Result<R> {
    let attachment = lua
        .app_data_ref::<AttachedNativeCall>()
        .map(|attachment| attachment.clone())
        .ok_or_else(missing)?;
    attachment.with_session(f)
}

pub(crate) fn with_attached_simulation_context<R>(
    lua: &Lua,
    f: impl FnOnce(&robin_engine::sim_rng::SimulationContext) -> mlua::Result<R>,
) -> mlua::Result<R> {
    with_attached_session(
        lua,
        || {
            mlua::Error::RuntimeError(
                "robin_lua: random invoked with no simulation context attached".to_owned(),
            )
        },
        |session| f(session.capabilities.simulation_context()),
    )
}

fn with_attached_bindings<R>(
    lua: &Lua,
    f: impl FnOnce(&AttachedScriptBindings) -> mlua::Result<R>,
) -> mlua::Result<R> {
    with_attached_session(
        lua,
        || {
            mlua::Error::RuntimeError(
                "robin_lua: native invoked with no ScriptBindings attached".to_owned(),
            )
        },
        |session| f(session.bindings),
    )
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
        with_attached_session(
            lua,
            || {
                mlua::Error::RuntimeError(format!(
                    "{}: called with no GameHost attached",
                    sig.name
                ))
            },
            |session| {
                let mut native_context = session.native_context();
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
                match robin_engine::interp::HostFunctions::call(
                    &mut native_context,
                    index,
                    &mut stack,
                ) {
                    NativeCallOutcome::Return(ret) => {
                        Ok(return_from_stack_word(ret, return_type))
                    }
                    NativeCallOutcome::PendingNestedCall(call) => {
                        Err(mlua::Error::RuntimeError(format!(
                            "{} requires nested script dispatch, which is unavailable through the Lua host adapter: {call:?}",
                            sig.name
                        )))
                    }
                }
            },
        )
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
        with_attached_bindings(lua, |bindings| {
            Ok(bindings.lua_names.actors.get(&name).copied().unwrap_or(0))
        })
    })?;
    globals.set("GetActor", get_actor)?;

    let get_item = lua.create_function(|lua, name: String| {
        with_attached_bindings(lua, |bindings| {
            Ok(bindings.lua_names.items.get(&name).copied().unwrap_or(0))
        })
    })?;
    globals.set("GetItem", get_item)?;

    let get_location = lua.create_function(|lua, name: String| {
        with_attached_bindings(lua, |bindings| {
            Ok(bindings
                .lua_names
                .locations
                .get(&name)
                .copied()
                .unwrap_or(0))
        })
    })?;
    globals.set("GetLocation", get_location)?;

    let get_patrol = lua.create_function(|lua, name: String| {
        with_attached_bindings(lua, |bindings| {
            Ok(bindings.lua_names.patrols.get(&name).copied().unwrap_or(0))
        })
    })?;
    globals.set("GetPatrol", get_patrol)?;

    let get_scroll = lua.create_function(|lua, name: String| {
        with_attached_bindings(lua, |bindings| {
            Ok(bindings.lua_names.scrolls.get(&name).copied().unwrap_or(0))
        })
    })?;
    globals.set("GetScroll", get_scroll)?;

    // ── Reverse lookup: handle → name ──
    let get_actor_name = lua.create_function(|lua, handle: i32| {
        with_attached_bindings(lua, |bindings| {
            // Linear scan — Spellforge's DLL does the same. The maps
            // are mission-scoped (low hundreds of entries), so this
            // doesn't merit a reverse index.
            for (name, h) in &bindings.lua_names.actors {
                if *h == handle {
                    return Ok(name.clone());
                }
            }
            // Spellforge returns the literal "<not found>" sentinel
            // when no name matches — preserved here for script parity.
            Ok("<not found>".to_owned())
        })
    })?;
    globals.set("GetActorName", get_actor_name)?;

    // ── Whole-table dumps ──
    //
    // Used by Spellforge's `lib/common.lua` to iterate every named
    // actor and assign patrols / cutscene roles in bulk.
    let get_all_actors = lua.create_function(|lua, ()| {
        with_attached_bindings(lua, |bindings| {
            let table = lua.create_table_with_capacity(0, bindings.lua_names.actors.len())?;
            for (name, handle) in &bindings.lua_names.actors {
                table.set(name.clone(), *handle)?;
            }
            Ok(table)
        })
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
        with_attached_session(
            lua,
            || {
                mlua::Error::RuntimeError(
                    "robin_lua: SequenceCall invoked with no native session attached".to_owned(),
                )
            },
            |session| {
                let mut native_context = session.native_context();
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
            },
        )
    })?;
    globals.set("SequenceCall", sequence_call)?;

    Ok(())
}

// Canonical Lua enumeration is declared by
// `robin_engine::natives::NATIVE_REGISTRY`; see `register_natives`.

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::entities::Entities;

    #[test]
    fn overlapping_session_reborrow_is_rejected_and_released() {
        let mut host = GameHost::new();
        let mut script_state = ScriptState::default();
        let mut script_domains = ScriptDomains::default();
        let bindings = AttachedScriptBindings::default();
        let mut entities = Entities::new();
        let mut ai_global = robin_engine::ai::AiGlobalState::default();
        let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
        let simulation = robin_engine::sim_rng::SimulationContext::with_seed(1);
        let capabilities = NativeSessionCapabilities::new(
            &simulation,
            &mut entities,
            &mut ai_global,
            &mut fast_grid,
        );
        let mut session = NativeCallSession::new(
            &mut host,
            &mut script_state,
            &mut script_domains,
            &bindings,
            &capabilities,
        );
        let attachment = AttachedNativeCall::new(&mut session);

        attachment
            .with_session(|_session| {
                let error = attachment
                    .with_session(|_nested| Ok(()))
                    .expect_err("overlapping reborrow must fail");
                assert!(error.to_string().contains("already in use"));
                Ok(())
            })
            .expect("outer reborrow");

        attachment
            .with_session(|_session| Ok(()))
            .expect("in-use guard must clear after the outer call");

        attachment.invalidate();
        let error = attachment
            .with_session(|_session| Ok(()))
            .expect_err("invalidated attachment must not dereference its session");
        assert!(error.to_string().contains("no longer attached"));
    }
}
