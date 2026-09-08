//! Process-local activation state and VM construction.

use crate::{PACKAGE_SOURCE_LIMIT, package};
mod abi;
mod bridge;
mod replay;
pub use abi::{spellforge_vm_abi, spellforge_vm_abi_digest};
use bridge::{BOOTSTRAP, native_bridge_source};
use replay::replay_event;

use rilua::vm::state::LuaState;
use rilua::{
    Function, IntoLua, Lua, LuaApi, LuaApiMut, LuaError, RuntimeError, StdLib, Table, Val,
};
use robin_engine::natives::{
    NATIVE_REGISTRY, NativeAbiType, NativeFn, NativeNamespace, SPELLFORGE_NATIVE_ALIASES,
    ScriptNameBindings,
};
use robin_engine::spellforge::{
    SPELLFORGE_ARGUMENT_WORD_LIMIT, SPELLFORGE_CONTRACT_VERSION,
    SPELLFORGE_EVENT_NATIVE_CALL_LIMIT, SPELLFORGE_EVENT_TRANSCRIPT_ENTRY_LIMIT,
    SPELLFORGE_SNAPSHOT_BYTE_LIMIT, SPELLFORGE_TAPE_BYTE_LIMIT, SPELLFORGE_TAPE_EVENT_LIMIT,
    SPELLFORGE_TAPE_NATIVE_CALL_LIMIT, SPELLFORGE_TAPE_STRING_BYTE_LIMIT,
    SPELLFORGE_TAPE_TRANSCRIPT_ENTRY_LIMIT, SPELLFORGE_VM_ABI_SCHEME, SpellforgeEventRecord,
    SpellforgeGuestError, SpellforgeGuestErrorKind, SpellforgeInvocation, SpellforgeJournal,
    SpellforgeNativeCall, SpellforgeNestedTapeEntry, SpellforgePackage, SpellforgeRuntime,
    SpellforgeStep, SpellforgeTape, SpellforgeTarget, SpellforgeTraceFrame,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Mutex, MutexGuard};

const MEMORY_LIMIT: usize = PACKAGE_SOURCE_LIMIT;
const INSTRUCTION_LIMIT: u32 = 1_000_000;
const BRIDGE_MARKER: &[u8] = b"__robin_native_v1";
const BUDGET_ERROR: &str = "Spellforge execution budget exceeded";
const PSEUDO_NATIVE_RANDOM: u32 = u32::MAX;
type SpellforgeResult<T> = Result<T, SpellforgeGuestError>;

struct Driver {
    has_handler: Function,
    begin: Function,
    resume: Function,
}

struct RuntimeVm {
    lua: Lua,
    driver: Driver,
    next_replay_activation: u64,
}

struct Activation {
    invocation: SpellforgeInvocation,
    parent_activation: Option<u64>,
    native_calls: Vec<SpellforgeNativeCall>,
    state: ActivationState,
    usage: InFlightUsage,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PendingNative {
    native_index: u32,
    arguments: Vec<i32>,
    nested_transcript: Vec<SpellforgeNestedTapeEntry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
enum ActivationState {
    Waiting(PendingNative),
    Complete(i32),
}

impl ActivationState {
    fn pending(&self) -> Option<&PendingNative> {
        match self {
            Self::Waiting(pending) => Some(pending),
            Self::Complete(_) => None,
        }
    }
    fn pending_mut(&mut self) -> Option<&mut PendingNative> {
        match self {
            Self::Waiting(pending) => Some(pending),
            Self::Complete(_) => None,
        }
    }
}

/// Counts the same completed-native and nested-entry resources as tape
/// admission. Pending native requests have not completed and do not count yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct InFlightUsage {
    native_calls: usize,
    transcript_entries: usize,
}

impl InFlightUsage {
    fn add(self, other: Self) -> SpellforgeResult<Self> {
        let next = Self {
            native_calls: self
                .native_calls
                .checked_add(other.native_calls)
                .ok_or_else(|| resource_limit("Spellforge in-flight native count overflow"))?,
            transcript_entries: self
                .transcript_entries
                .checked_add(other.transcript_entries)
                .ok_or_else(|| resource_limit("Spellforge in-flight transcript count overflow"))?,
        };
        if next.native_calls > SPELLFORGE_TAPE_NATIVE_CALL_LIMIT as usize {
            return Err(resource_limit(
                "Spellforge in-flight native call limit exceeded",
            ));
        }
        if next.transcript_entries > SPELLFORGE_TAPE_TRANSCRIPT_ENTRY_LIMIT as usize {
            return Err(resource_limit(
                "Spellforge in-flight transcript entry limit exceeded",
            ));
        }
        Ok(next)
    }

    fn remove(self, other: Self) -> Self {
        Self {
            native_calls: self
                .native_calls
                .checked_sub(other.native_calls)
                .expect("in-flight native accounting underflow"),
            transcript_entries: self
                .transcript_entries
                .checked_sub(other.transcript_entries)
                .expect("in-flight transcript accounting underflow"),
        }
    }
}

struct RuntimeInner {
    vm: RuntimeVm,
    names: ScriptNameBindings,
    applied_tape: SpellforgeJournal,
    next_activation: u64,
    activations: HashMap<u64, Activation>,
    needs_rebuild: bool,
    usage: InFlightUsage,
}

impl RuntimeInner {
    /// Discard every uncommitted activation. The next entry reconstructs the
    /// heap solely from committed history, even if callers omit abort.
    fn invalidate(&mut self) {
        self.activations.clear();
        self.usage = InFlightUsage::default();
        self.needs_rebuild = true;
    }
}

/// The gameplay Spellforge runtime used on every supported target.
pub struct SpellforgeRuntime51 {
    package: SpellforgePackage,
    inner: Mutex<RuntimeInner>,
}

impl SpellforgeRuntime51 {
    fn lock_runtime(&self) -> MutexGuard<'_, RuntimeInner> {
        match self.inner.lock() {
            Ok(inner) => inner,
            Err(poisoned) => {
                // A panic may have interrupted any transition. Never expose
                // that heap as if it still represented the committed tape.
                let mut inner = poisoned.into_inner();
                inner.invalidate();
                self.inner.clear_poison();
                inner
            }
        }
    }

    pub fn new(package: SpellforgePackage) -> SpellforgeResult<Self> {
        validate_package(&package)?;
        let names = ScriptNameBindings::default();
        let vm = build_vm(&package, &names)?;
        Ok(Self {
            package,
            inner: Mutex::new(RuntimeInner {
                vm,
                names,
                applied_tape: SpellforgeJournal::default(),
                next_activation: 1,
                activations: HashMap::new(),
                needs_rebuild: false,
                usage: InFlightUsage::default(),
            }),
        })
    }

    fn synchronize(&self, inner: &mut RuntimeInner, tape: &SpellforgeTape) -> SpellforgeResult<()> {
        let installed = tape
            .package
            .as_deref()
            .ok_or_else(|| "Spellforge engine tape has no installed package".to_owned())?;
        if installed != &self.package {
            return Err(format!(
                "Spellforge runtime package {} does not exactly match tape package {}",
                robin_engine::spellforge::hex_hash(&self.package.sha256),
                robin_engine::spellforge::hex_hash(&installed.sha256),
            )
            .into());
        }
        if !inner.activations.is_empty() {
            if !inner.needs_rebuild && inner.applied_tape == tape.events {
                return Ok(());
            }
            return Err(
                "Spellforge tape or bindings changed while a Lua event was suspended".into(),
            );
        }
        if !inner.needs_rebuild && inner.applied_tape == tape.events {
            return Ok(());
        }

        let mut rebuilt = build_vm(&self.package, &inner.names)?;
        for (event_index, record) in tape.events.iter().enumerate() {
            let result = replay_event(&mut rebuilt, record).map_err(|error| {
                SpellforgeGuestError::new(
                    SpellforgeGuestErrorKind::Divergence,
                    format!("Spellforge tape divergence at event {event_index}: {error}"),
                )
            })?;
            if result != record.result {
                return Err(SpellforgeGuestError::new(
                    SpellforgeGuestErrorKind::Divergence,
                    format!(
                        "Spellforge tape divergence at event {event_index}: expected result {}, rebuilt {result}",
                        record.result
                    ),
                ));
            }
        }
        inner.vm = rebuilt;
        inner.applied_tape = tape.events.clone();
        inner.needs_rebuild = false;
        Ok(())
    }
}

impl SpellforgeRuntime for SpellforgeRuntime51 {
    fn package(&self) -> &SpellforgePackage {
        &self.package
    }

    fn set_name_bindings(&self, names: ScriptNameBindings) {
        let mut inner = self.lock_runtime();
        if !same_names(&inner.names, &names) {
            inner.names = names;
            inner.needs_rebuild = true;
        }
    }

    fn has_handler(
        &self,
        invocation: &SpellforgeInvocation,
        tape: &SpellforgeTape,
    ) -> SpellforgeResult<bool> {
        let mut inner = self.lock_runtime();
        self.synchronize(&mut inner, tape)?;
        has_handler(&mut inner.vm, invocation)
    }

    fn begin(
        &self,
        invocation: SpellforgeInvocation,
        tape: &SpellforgeTape,
    ) -> SpellforgeResult<SpellforgeStep> {
        let mut inner = self.lock_runtime();
        let outcome = (|| {
            self.synchronize(&mut inner, tape)?;
            validate_live_invocation(&invocation)?;
            if inner
                .activations
                .values()
                .any(|active| matches!(active.state, ActivationState::Complete(_)))
            {
                return Err(SpellforgeGuestError::protocol(
                    "Spellforge completed activation must commit before beginning another event",
                ));
            }
            let activation = inner.next_activation;
            inner.next_activation = activation.checked_add(1).ok_or_else(|| {
                SpellforgeGuestError::protocol("Spellforge activation id overflow")
            })?;
            let parent_activation = inner
                .activations
                .iter()
                .filter(|(_, active)| active.state.pending().is_some())
                .map(|(id, _)| *id)
                .max();
            let step = begin_event(&mut inner.vm, activation, &invocation)?;
            let state = step_state(&step)?;
            inner.activations.insert(
                activation,
                Activation {
                    invocation: invocation.clone(),
                    parent_activation,
                    native_calls: Vec::new(),
                    state,
                    usage: InFlightUsage::default(),
                },
            );
            Ok(step)
        })();
        if outcome.is_err() {
            inner.invalidate();
        }
        outcome.map_err(|mut error: SpellforgeGuestError| {
            if error.invocation.is_none() {
                error.invocation = Some(invocation);
            }
            error
        })
    }

    fn resume(
        &self,
        activation: u64,
        return_word: i32,
        tape: &SpellforgeTape,
    ) -> SpellforgeResult<SpellforgeStep> {
        let mut inner = self.lock_runtime();
        let invocation = inner
            .activations
            .get(&activation)
            .map(|active| active.invocation.clone());
        let outcome = (|| {
            if inner.applied_tape != tape.events || inner.needs_rebuild {
                return Err(SpellforgeGuestError::protocol(
                    "Spellforge tape changed while a native was suspended",
                ));
            }
            let active = inner.activations.get(&activation).ok_or_else(|| {
                SpellforgeGuestError::protocol(format!(
                    "unknown Spellforge activation {activation}"
                ))
            })?;
            if active.native_calls.len() >= SPELLFORGE_EVENT_NATIVE_CALL_LIMIT {
                return Err(resource_limit(format!(
                    "Spellforge event reached the {SPELLFORGE_EVENT_NATIVE_CALL_LIMIT} direct-native-call limit"
                )));
            }
            let pending = active.state.pending().ok_or_else(|| {
                SpellforgeGuestError::protocol(format!(
                    "Spellforge activation {activation} is not waiting for a native"
                ))
            })?;
            validate_live_arguments("native request", &pending.arguments)?;
            if pending.nested_transcript.len() > SPELLFORGE_EVENT_TRANSCRIPT_ENTRY_LIMIT {
                return Err(resource_limit(
                    "Spellforge event nested transcript limit exceeded",
                ));
            }
            if inner
                .activations
                .values()
                .any(|child| child.parent_activation == Some(activation))
            {
                return Err(SpellforgeGuestError::protocol(
                    "Spellforge nested activation must commit before its parent resumes",
                ));
            }
            let increment = InFlightUsage {
                native_calls: 1,
                transcript_entries: 0,
            };
            let total = inner.usage.add(increment)?;
            let usage = active.usage.add(increment)?;
            // Do not consume the pending request until execution and validation
            // succeed. Any error invalidates the whole uncommitted heap below.
            let step = resume_event(&mut inner.vm, activation, return_word)?;
            let next = step_state(&step)?;
            let active = inner
                .activations
                .get_mut(&activation)
                .expect("activation exists");
            let ActivationState::Waiting(pending) = std::mem::replace(&mut active.state, next)
            else {
                unreachable!("pending request checked before resume");
            };
            active.native_calls.push(SpellforgeNativeCall {
                native_index: pending.native_index,
                arguments: pending.arguments,
                return_word,
                nested_transcript: pending.nested_transcript,
            });
            active.usage = usage;
            inner.usage = total;
            Ok(step)
        })();
        if outcome.is_err() {
            inner.invalidate();
        }
        outcome.map_err(|mut error: SpellforgeGuestError| {
            if error.invocation.is_none() {
                error.invocation = invocation;
            }
            error
        })
    }

    fn commit(
        &self,
        activation: u64,
        result: i32,
        tape: &mut SpellforgeTape,
    ) -> SpellforgeResult<()> {
        let mut inner = self.lock_runtime();
        let outcome = (|| {
            if inner.applied_tape != tape.events || inner.needs_rebuild {
                return Err(SpellforgeGuestError::protocol(
                    "Spellforge tape changed before event commit",
                ));
            }
            let active = inner.activations.get(&activation).ok_or_else(|| {
                SpellforgeGuestError::protocol(format!(
                    "unknown Spellforge activation {activation}"
                ))
            })?;
            match active.state {
                ActivationState::Waiting(_) => {
                    return Err(SpellforgeGuestError::protocol(format!(
                        "Spellforge activation {activation} committed while waiting for a native"
                    )));
                }
                ActivationState::Complete(expected) if expected != result => {
                    return Err(SpellforgeGuestError::protocol(format!(
                        "Spellforge activation {activation} result changed from {expected} to {result}"
                    )));
                }
                ActivationState::Complete(_) => {}
            }
            // From this point errors invalidate all uncommitted state. In
            // particular, a failed tape append cannot leave a mutated Lua heap
            // that falsely claims to match the old tape.
            let active = inner
                .activations
                .remove(&activation)
                .expect("validated activation");
            let remaining = inner.usage.remove(active.usage);
            let record = SpellforgeEventRecord {
                invocation: active.invocation,
                native_calls: active.native_calls,
                result,
            };
            if let Some(parent_activation) = active.parent_activation {
                let parent = inner.activations.get_mut(&parent_activation).ok_or_else(||
                    SpellforgeGuestError::protocol(format!("Spellforge nested activation {activation} lost parent {parent_activation}")))?;
                let pending = parent.state.pending_mut().ok_or_else(||
                    SpellforgeGuestError::protocol(format!("Spellforge nested activation {activation} parent {parent_activation} is no longer suspended")))?;
                let before = pending.nested_transcript.len();
                append_nested_record(&mut pending.nested_transcript, record)?;
                let increment = InFlightUsage {
                    native_calls: 0,
                    transcript_entries: pending.nested_transcript.len() - before,
                };
                parent.usage = parent.usage.add(increment)?;
                inner.usage = remaining.add(increment)?;
            } else {
                tape.append_event(record)?;
                inner.applied_tape = tape.events.clone();
                inner.usage = remaining;
            }
            Ok(())
        })();
        if outcome.is_err() {
            inner.invalidate();
        }
        outcome
    }

    fn abort(&self, activation: u64) {
        let mut inner = self.lock_runtime();
        if inner.activations.contains_key(&activation) {
            inner.invalidate();
        }
    }
}

fn same_names(left: &ScriptNameBindings, right: &ScriptNameBindings) -> bool {
    left.actors == right.actors
        && left.items == right.items
        && left.locations == right.locations
        && left.patrols == right.patrols
        && left.scrolls == right.scrolls
}

fn step_state(step: &SpellforgeStep) -> SpellforgeResult<ActivationState> {
    Ok(match step {
        SpellforgeStep::Native {
            native_index,
            arguments,
            ..
        } => {
            validate_live_arguments("native request", arguments)?;
            ActivationState::Waiting(PendingNative {
                native_index: *native_index,
                arguments: arguments.clone(),
                nested_transcript: Vec::new(),
            })
        }
        SpellforgeStep::Complete { result, .. } => ActivationState::Complete(*result),
    })
}

fn append_nested_record(
    transcript: &mut Vec<SpellforgeNestedTapeEntry>,
    record: SpellforgeEventRecord,
) -> SpellforgeResult<()> {
    validate_live_invocation(&record.invocation)?;
    if record.native_calls.len() > SPELLFORGE_EVENT_NATIVE_CALL_LIMIT {
        return Err(resource_limit(format!(
            "Spellforge nested event contains {} direct native calls; limit is {SPELLFORGE_EVENT_NATIVE_CALL_LIMIT}",
            record.native_calls.len()
        )));
    }
    let additional = record
        .native_calls
        .iter()
        .try_fold(2usize, |total, native| {
            validate_live_arguments("nested native request", &native.arguments)?;
            total
                .checked_add(2)
                .and_then(|total| total.checked_add(native.nested_transcript.len()))
                .ok_or_else(|| resource_limit("Spellforge nested transcript length overflow"))
        })?;
    let prospective = transcript
        .len()
        .checked_add(additional)
        .ok_or_else(|| resource_limit("Spellforge nested transcript length overflow"))?;
    if prospective > SPELLFORGE_EVENT_TRANSCRIPT_ENTRY_LIMIT {
        return Err(resource_limit(format!(
            "Spellforge event would contain {prospective} nested transcript entries; limit is {SPELLFORGE_EVENT_TRANSCRIPT_ENTRY_LIMIT}"
        )));
    }
    transcript.push(SpellforgeNestedTapeEntry::Begin(record.invocation));
    for native in record.native_calls {
        transcript.push(SpellforgeNestedTapeEntry::NativeRequest {
            native_index: native.native_index,
            arguments: native.arguments,
        });
        transcript.extend(native.nested_transcript);
        transcript.push(SpellforgeNestedTapeEntry::NativeReturn {
            return_word: native.return_word,
        });
    }
    transcript.push(SpellforgeNestedTapeEntry::Complete {
        result: record.result,
    });
    Ok(())
}

fn validate_live_invocation(invocation: &SpellforgeInvocation) -> SpellforgeResult<()> {
    robin_engine::spellforge::validate_invocation(invocation).map_err(resource_limit)
}

fn validate_live_arguments(label: &str, arguments: &[i32]) -> SpellforgeResult<()> {
    robin_engine::spellforge::validate_arguments(label, arguments).map_err(resource_limit)
}

fn resource_limit(message: impl Into<String>) -> SpellforgeGuestError {
    SpellforgeGuestError::new(SpellforgeGuestErrorKind::ResourceLimit, message)
}

/// Validate an exact serialized package against this build's executable ABI
/// without invoking mission top-level code. Replay and network admission use
/// this before constructing the disposable runtime.
pub fn validate_package(package: &SpellforgePackage) -> SpellforgeResult<()> {
    validate_package_inner(package).map_err(|message| {
        SpellforgeGuestError::new(SpellforgeGuestErrorKind::Compatibility, message)
    })
}

fn validate_package_inner(package: &SpellforgePackage) -> Result<(), String> {
    package.validate_wire()?;
    if package.vm_abi != spellforge_vm_abi() {
        return Err(format!(
            "unsupported Spellforge VM ABI `{}`; expected `{}`",
            package.vm_abi,
            spellforge_vm_abi(),
        ));
    }
    Ok(())
}

/// Hash the exact versioned package identity used by saves, replays and peers.
pub fn compute_package_sha256(package: &SpellforgePackage) -> [u8; 32] {
    package.computed_sha256()
}

fn build_vm(
    package: &SpellforgePackage,
    names: &ScriptNameBindings,
) -> SpellforgeResult<RuntimeVm> {
    validate_package(package)?;
    let libs = StdLib::BASE
        | StdLib::TABLE
        | StdLib::STRING
        | StdLib::MATH
        | StdLib::COROUTINE
        | StdLib::DEBUG;
    let mut lua = Lua::new_with(libs).map_err(lua_error)?;
    lua.state_mut().gc.set_alloc_limit(MEMORY_LIMIT);
    lua.register_function("__robin_pack_f32", pack_f32)
        .map_err(lua_error)?;
    lua.register_function("__robin_unpack_f32", unpack_f32)
        .map_err(lua_error)?;

    let modules = lua.create_table();
    let aliases = lua.create_table();
    let mut module_aliases = BTreeMap::<String, String>::new();
    let canonical_modules = package
        .files
        .keys()
        .filter(|path| *path != &package.entrypoint && path.ends_with(".lua"))
        .map(|path| path.trim_end_matches(".lua").to_owned())
        .collect::<BTreeSet<_>>();
    for (path, bytes) in &package.files {
        if path == &package.entrypoint || !path.ends_with(".lua") {
            continue;
        }
        let canonical = path.trim_end_matches(".lua").to_owned();
        let function = lua
            .load_bytes(bytes, &format!("@{path}"))
            .map_err(lua_error)?;
        let function = function.into_lua(&mut lua).map_err(lua_error)?;
        table_set_string(&mut lua, modules, &canonical, function)?;
        insert_module_alias(&mut module_aliases, canonical.clone(), &canonical)?;
        insert_module_alias(&mut module_aliases, canonical.replace('/', "."), &canonical)?;
    }
    for canonical in &canonical_modules {
        let leaf = canonical.rsplit('/').next().unwrap_or(canonical);
        // A root module is the exact spelling requested by published
        // Spellforge packages and therefore wins over a convenience leaf
        // alias such as `lib/enums` -> `enums`.
        if leaf == canonical || canonical_modules.contains(leaf) {
            continue;
        }
        insert_module_alias(&mut module_aliases, leaf.to_owned(), canonical)?;
    }
    for (alias, canonical) in module_aliases {
        let value = lua.create_string(canonical.as_bytes());
        table_set_string(&mut lua, aliases, &alias, value)?;
    }
    lua.set_global("__robin_modules", modules)
        .map_err(lua_error)?;
    lua.set_global("__robin_module_aliases", aliases)
        .map_err(lua_error)?;
    let entry = lua
        .load_bytes(
            package
                .files
                .get(&package.entrypoint)
                .expect("validated entrypoint must exist"),
            &format!("@{}", package.entrypoint),
        )
        .map_err(lua_error)?;
    lua.set_global("__robin_entry", entry).map_err(lua_error)?;
    lua.set_global("__robin_instruction_limit", INSTRUCTION_LIMIT)
        .map_err(lua_error)?;

    install_name_tables(&mut lua, names)?;
    lua.exec(&native_bridge_source()).map_err(lua_error)?;
    lua.exec(BOOTSTRAP).map_err(lua_error)?;

    let driver_table: Table = lua.global("__robin_driver").map_err(lua_error)?;
    let driver = Driver {
        has_handler: table_function(&mut lua, driver_table, "has_handler")?,
        begin: table_function(&mut lua, driver_table, "begin")?,
        resume: table_function(&mut lua, driver_table, "resume")?,
    };
    // Root the private driver table in the inaccessible registry. Rust handles
    // are checked arena indices, not GC roots by themselves.
    let registry = Table::from_gc_ref(lua.state().registry);
    let key = lua.create_string(b"robin.spellforge.driver");
    registry
        .raw_set(lua.state_mut(), key, Val::Table(driver_table.gc_ref()))
        .map_err(lua_error)?;
    lua.set_global("__robin_driver", Val::Nil)
        .map_err(lua_error)?;
    Ok(RuntimeVm {
        lua,
        driver,
        next_replay_activation: 1 << 52,
    })
}

fn insert_module_alias(
    aliases: &mut BTreeMap<String, String>,
    alias: String,
    canonical: &str,
) -> SpellforgeResult<()> {
    if let Some(previous) = aliases.insert(alias.clone(), canonical.to_owned())
        && previous != canonical
    {
        return Err(format!(
            "Spellforge module alias `{alias}` is ambiguous between `{previous}` and `{canonical}`"
        )
        .into());
    }
    Ok(())
}

fn install_name_tables(lua: &mut Lua, names: &ScriptNameBindings) -> SpellforgeResult<()> {
    let root = lua.create_table();
    for (kind, entries) in [
        ("actors", &names.actors),
        ("items", &names.items),
        ("locations", &names.locations),
        ("patrols", &names.patrols),
        ("scrolls", &names.scrolls),
    ] {
        let table = lua.create_table();
        for (name, handle) in entries {
            table_set_string(lua, table, name, Val::Num(f64::from(*handle)))?;
        }
        table_set_string(lua, root, kind, Val::Table(table.gc_ref()))?;
    }
    lua.set_global("__robin_names", root).map_err(lua_error)
}

fn has_handler(vm: &mut RuntimeVm, invocation: &SpellforgeInvocation) -> SpellforgeResult<bool> {
    let (kind, class, message) = target_words(invocation);
    let args = [
        Val::Num(kind),
        vm.lua.create_string(class.as_bytes()),
        vm.lua.create_string(invocation.event.as_bytes()),
        message.map_or(Val::Nil, |value| Val::Num(f64::from(value))),
    ];
    let values = vm
        .lua
        .call_function(&vm.driver.has_handler, &args)
        .map_err(lua_error)?;
    match values.first() {
        Some(Val::Num(0.0)) => Ok(false),
        Some(Val::Num(1.0)) => Ok(true),
        Some(Val::Num(2.0)) => Err(format!(
            "Spellforge handler {:?}.{} exists but is not a function",
            invocation.target, invocation.event
        )
        .into()),
        value => Err(format!("Spellforge handler probe returned invalid value {value:?}").into()),
    }
}

fn begin_event(
    vm: &mut RuntimeVm,
    activation: u64,
    invocation: &SpellforgeInvocation,
) -> SpellforgeResult<SpellforgeStep> {
    let (kind, class, message) = target_words(invocation);
    let mut args = vec![
        Val::Num(activation as f64),
        Val::Num(kind),
        vm.lua.create_string(class.as_bytes()),
        vm.lua.create_string(invocation.event.as_bytes()),
        message.map_or(Val::Nil, |value| Val::Num(f64::from(value))),
    ];
    args.extend(
        invocation
            .args
            .iter()
            .map(|value| Val::Num(f64::from(*value))),
    );
    let values = vm
        .lua
        .call_function(&vm.driver.begin, &args)
        .map_err(normalize_runtime_error)?;
    parse_step(&vm.lua, activation, values)
}

fn resume_event(
    vm: &mut RuntimeVm,
    activation: u64,
    word: i32,
) -> SpellforgeResult<SpellforgeStep> {
    let values = vm
        .lua
        .call_function(
            &vm.driver.resume,
            &[Val::Num(activation as f64), Val::Num(f64::from(word))],
        )
        .map_err(normalize_runtime_error)?;
    parse_step(&vm.lua, activation, values)
}

fn target_words(invocation: &SpellforgeInvocation) -> (f64, &str, Option<i32>) {
    let (kind, class) = match &invocation.target {
        SpellforgeTarget::Global => (0.0, ""),
        SpellforgeTarget::Actor { class, .. } => (1.0, class.as_str()),
        SpellforgeTarget::Target { class, .. } => (2.0, class.as_str()),
        SpellforgeTarget::Scroll { class, .. } => (3.0, class.as_str()),
        SpellforgeTarget::Zone { class, .. } => (4.0, class.as_str()),
        SpellforgeTarget::Waypoint { class, .. } => (5.0, class.as_str()),
    };
    let message = (matches!(invocation.target, SpellforgeTarget::Global)
        && invocation.event == "ProcessMessage")
        .then(|| invocation.args.first().copied())
        .flatten();
    (kind, class, message)
}

fn parse_step(lua: &Lua, activation: u64, values: Vec<Val>) -> SpellforgeResult<SpellforgeStep> {
    let Some(done) = values.first() else {
        return Err("Spellforge driver returned no completion flag".into());
    };
    match done {
        Val::Bool(true) => Ok(SpellforgeStep::Complete {
            activation,
            result: parse_result(values.get(1).copied())?,
        }),
        Val::Bool(false) => {
            let marker = values
                .get(1)
                .and_then(|value| lua.val_as_bytes(*value))
                .ok_or_else(|| "Spellforge coroutine yielded without bridge marker".to_owned())?;
            if marker != BRIDGE_MARKER {
                return Err("Spellforge coroutine yielded an unsupported marker".into());
            }
            let native_index = parse_u32(values.get(2).copied(), "native index")?;
            let Val::Table(words_ref) = values.get(3).copied().unwrap_or(Val::Nil) else {
                return Err("Spellforge coroutine yielded without native argument table".into());
            };
            if values.len() != 4 {
                return Err("Spellforge coroutine yielded unexpected extra values".into());
            }
            let words = Table::from_gc_ref(words_ref);
            let mut arguments = Vec::with_capacity(words.raw_len(lua.state()) as usize);
            for index in 1..=words.raw_len(lua.state()) {
                let value = words
                    .raw_get(lua.state(), Val::Num(index as f64))
                    .map_err(lua_error)?;
                arguments.push(parse_i32(Some(value), "native argument")?);
            }
            Ok(SpellforgeStep::Native {
                activation,
                native_index,
                arguments,
            })
        }
        _ => Err("Spellforge driver returned a non-boolean completion flag".into()),
    }
}

fn parse_result(value: Option<Val>) -> SpellforgeResult<i32> {
    match value.unwrap_or(Val::Nil) {
        Val::Nil => Ok(0),
        Val::Bool(value) => Ok(i32::from(value)),
        Val::Num(value)
            if value.is_finite()
                && value.fract() == 0.0
                && value >= i32::MIN as f64
                && value <= i32::MAX as f64 =>
        {
            Ok(value as i32)
        }
        value => Err(format!(
            "Spellforge event returned Lua {}; expected integer, boolean, or nil",
            value.type_name()
        )
        .into()),
    }
}

fn table_set_string(lua: &mut Lua, table: Table, key: &str, value: Val) -> SpellforgeResult<()> {
    let key = lua.create_string(key.as_bytes());
    table
        .raw_set(lua.state_mut(), key, value)
        .map_err(lua_error)
}

fn table_function(lua: &mut Lua, table: Table, key: &str) -> SpellforgeResult<Function> {
    let key = lua.create_string(key.as_bytes());
    match table.raw_get(lua.state(), key).map_err(lua_error)? {
        Val::Function(function) => Ok(Function::from_gc_ref(function)),
        value => Err(format!(
            "Spellforge private driver `{key:?}` is Lua {}",
            value.type_name()
        )
        .into()),
    }
}

fn parse_i32(value: Option<Val>, what: &str) -> SpellforgeResult<i32> {
    let Val::Num(value) = value.unwrap_or(Val::Nil) else {
        return Err(format!("Spellforge {what} is not a number").into());
    };
    if !value.is_finite()
        || value.fract() != 0.0
        || value < i32::MIN as f64
        || value > i32::MAX as f64
    {
        return Err(format!("Spellforge {what} {value} is outside i32").into());
    }
    Ok(value as i32)
}

fn parse_u32(value: Option<Val>, what: &str) -> SpellforgeResult<u32> {
    let Val::Num(value) = value.unwrap_or(Val::Nil) else {
        return Err(format!("Spellforge {what} is not a number").into());
    };
    if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value > u32::MAX as f64 {
        return Err(format!("Spellforge {what} {value} is outside u32").into());
    }
    Ok(value as u32)
}

fn pack_f32(state: &mut LuaState) -> rilua::LuaResult<u32> {
    let value = match state.stack_get(state.base) {
        Val::Num(value) => value,
        value => {
            return Err(runtime_error(format!(
                "expected number, got {}",
                value.type_name()
            )));
        }
    };
    let packed = value as f32;
    if !value.is_finite() || !packed.is_finite() {
        return Err(runtime_error(format!(
            "number {value} is not representable as finite f32"
        )));
    }
    state.stack_set(state.base, Val::Num(f64::from(packed.to_bits() as i32)));
    state.top = state.base + 1;
    Ok(1)
}

fn unpack_f32(state: &mut LuaState) -> rilua::LuaResult<u32> {
    let value = match state.stack_get(state.base) {
        Val::Num(value)
            if value.is_finite()
                && value.fract() == 0.0
                && value >= i32::MIN as f64
                && value <= i32::MAX as f64 =>
        {
            value as i32
        }
        _ => return Err(runtime_error("expected signed 32-bit float word".into())),
    };
    state.stack_set(state.base, Val::Num(f32::from_bits(value as u32) as f64));
    state.top = state.base + 1;
    Ok(1)
}

fn runtime_error(message: String) -> LuaError {
    LuaError::Runtime(RuntimeError {
        message,
        level: 0,
        traceback: Vec::new(),
    })
}

fn normalize_runtime_error(error: LuaError) -> SpellforgeGuestError {
    let mut failure = lua_error(error);
    if failure.message.contains(BUDGET_ERROR) {
        failure.kind = SpellforgeGuestErrorKind::Budget;
        failure.message = BUDGET_ERROR.to_owned();
    }
    failure
}

fn lua_error(error: LuaError) -> SpellforgeGuestError {
    match error {
        LuaError::Syntax(error) => SpellforgeGuestError {
            kind: SpellforgeGuestErrorKind::Syntax,
            message: error.message,
            invocation: None,
            traceback: vec![SpellforgeTraceFrame {
                source: error.source,
                line: error.line,
                function: None,
            }],
        },
        LuaError::Runtime(error) => SpellforgeGuestError {
            kind: SpellforgeGuestErrorKind::Runtime,
            message: error.message,
            invocation: None,
            traceback: error
                .traceback
                .into_iter()
                .map(|frame| SpellforgeTraceFrame {
                    source: frame.source,
                    line: frame.line,
                    function: frame.name,
                })
                .collect(),
        },
        LuaError::Memory => {
            SpellforgeGuestError::new(SpellforgeGuestErrorKind::Memory, "not enough memory")
        }
        LuaError::ErrorHandler => SpellforgeGuestError::new(
            SpellforgeGuestErrorKind::Internal,
            "error in Lua error handler",
        ),
        LuaError::Io(error) => SpellforgeGuestError::new(
            SpellforgeGuestErrorKind::Sandbox,
            format!("sandboxed Lua I/O failure: {error}"),
        ),
        LuaError::Yield(values) => SpellforgeGuestError::new(
            SpellforgeGuestErrorKind::Protocol,
            format!("unexpected raw Lua yield with {values} values"),
        ),
    }
}

#[cfg(test)]
mod tests;
