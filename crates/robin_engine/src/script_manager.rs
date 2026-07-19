//! Script manager: loads `.scb` bytecode, manages class-bound VM instances.
//!
//! One `ScriptManager` per loaded `.scb` (i.e., per mission level). All
//! `ScriptInstance`s created from it share the same program code and static
//! memory area (the 0x0000..0x3FFF symbol range).

use std::fmt;

use crate::interp::{Frame, HostFunctions, StopReason, Vm};
use crate::scb::{self, Function, ScbFile};
use crate::vm::{self, Instruction};

// ───────────────────────── Errors ─────────────────────────

/// Errors from script manager operations.
#[derive(Debug)]
pub enum ScriptError {
    /// The `.scb` file could not be loaded or parsed.
    Load(scb::Error),
    /// No class with this name exists in the loaded script.
    ClassNotFound(String),
    /// No function with this name exists in the bound class.
    FunctionNotFound(String),
    /// The VM stopped abnormally during execution.
    Vm(StopReason),
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScriptError::Load(e) => write!(f, "script load error: {e}"),
            ScriptError::ClassNotFound(name) => write!(f, "class not found: {name}"),
            ScriptError::FunctionNotFound(name) => write!(f, "function not found: {name}"),
            ScriptError::Vm(stop) => write!(f, "VM stopped abnormally: {stop:?}"),
        }
    }
}

impl std::error::Error for ScriptError {}

impl From<scb::Error> for ScriptError {
    fn from(e: scb::Error) -> Self {
        ScriptError::Load(e)
    }
}

// ───────────────────────── ScriptProgram ─────────────────────────

/// Immutable code & startup data loaded from a `.scb` file.
///
/// Split out of [`ScriptManager`] so rollback/network state-sync can
/// cheaply share the bytecode via [`Arc`] (free clone) while the
/// *mutable* script state — the shared static area, the per-instance
/// VM heaps — travels along the runtime path. Every client loads the
/// same `.scb` at match start, so the bytecode is identical across all
/// peers and never needs to cross the network.
///
/// `ScriptProgram` is immutable level data. `ScriptManager` snapshots store
/// only mutable script state and require the host to reattach this program
/// after deserialization.
#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct ScriptProgram {
    pub scb: ScbFile,
    /// Pre-decoded instruction streams, one per class, indexed in parallel
    /// with `scb.classes`.
    pub programs: Vec<Vec<Instruction>>,
}

/// A `ScriptProgram` with no classes — used as the placeholder when a
/// `ScriptManager` is deserialized without yet having the real bytecode
/// attached. Running any script against this will return
/// `ClassNotFound`, which is the intended failure mode when the host
/// forgets to call [`ScriptManager::attach_program`].
impl Default for ScriptProgram {
    fn default() -> Self {
        Self {
            scb: ScbFile {
                version: 0.0,
                classes: Vec::new(),
            },
            programs: Vec::new(),
        }
    }
}

impl ScriptProgram {
    /// Decode a parsed `.scb` file into a reusable `ScriptProgram`.
    pub fn from_scb(scb: ScbFile) -> Self {
        let programs = scb
            .classes
            .iter()
            .map(|class| {
                class
                    .quads
                    .iter()
                    .map(|q| vm::decode(*q).unwrap_or(Instruction::Empty))
                    .collect()
            })
            .collect();
        Self { scb, programs }
    }
}

// ───────────────────────── ScriptManager ─────────────────────────

/// Runtime wrapper around a loaded [`ScriptProgram`].
///
/// Holds an `Arc<ScriptProgram>` (shared, immutable code) plus the
/// mutable script state that varies at runtime: the shared static area
/// that all VM instances in a level read/write. Cloning is cheap — the
/// bytecode is an `Arc` bump, only the static area deep-copies.
///
/// Serialization carries only mutable VM state. Immutable bytecode is a
/// level asset and is reattached after decode through [`attach_program`].
#[derive(Clone)]
pub struct ScriptManager {
    /// Shared immutable bytecode + class metadata.
    pub program: std::sync::Arc<ScriptProgram>,
    /// Shared static area. The VM's 0x0000..0x3FFF symbol range reads/writes
    /// here — a single byte array shared by all VM instances in a level.
    pub static_area: Vec<u8>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ScriptManagerSnapshot {
    static_area: Vec<u8>,
}

impl serde::Serialize for ScriptManager {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        ScriptManagerSnapshot {
            static_area: self.static_area.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for ScriptManager {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let snapshot = ScriptManagerSnapshot::deserialize(deserializer)?;
        Ok(Self {
            program: std::sync::Arc::new(ScriptProgram::default()),
            static_area: snapshot.static_area,
        })
    }
}

impl robin_util::state_hash::StateHash for ScriptManager {
    fn state_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        robin_util::state_hash::StateHash::state_hash(&self.static_area, state);
    }
}

impl ScriptManager {
    /// Create a manager from an already-parsed `.scb` file.
    pub fn new(scb: ScbFile) -> Self {
        Self::from_program(std::sync::Arc::new(ScriptProgram::from_scb(scb)))
    }

    /// Create a manager from host-owned immutable script bytecode.
    pub fn from_program(program: std::sync::Arc<ScriptProgram>) -> Self {
        Self {
            program,
            static_area: vec![0u8; 4096],
        }
    }

    // NOTE: `load_file` / `load_bytes` used to live here but the parser
    // is in `robin_assets::scb`. Host callers should parse the file
    // there, then pass the `ScbFile` to `ScriptManager::new`. See
    // Decision 2 in the carve-out refactor.

    /// Re-attach a loaded `ScriptProgram` after deserialization.
    ///
    /// Serialized `ScriptManager`s arrive with a default (empty)
    /// program — the host must call this to bind the real bytecode
    /// loaded from the level's `.scb` before running any script.
    pub fn attach_program(&mut self, program: std::sync::Arc<ScriptProgram>) {
        self.program = program;
    }

    /// Number of classes in the loaded script.
    pub fn class_count(&self) -> usize {
        self.program.scb.classes.len()
    }

    /// Iterate over all class names.
    pub fn class_names(&self) -> impl Iterator<Item = &str> {
        self.program
            .scb
            .classes
            .iter()
            .map(|c| c.class_name.as_str())
    }

    /// Look up a class index by name. Returns `None` if not found.
    pub fn find_class(&self, name: &str) -> Option<usize> {
        self.program
            .scb
            .classes
            .iter()
            .position(|c| c.class_name == name)
    }

    /// Get the underlying ScbFile.
    pub fn scb(&self) -> &ScbFile {
        &self.program.scb
    }

    /// Create a new `ScriptInstance` bound to the named class.
    ///
    /// The instance gets its own heap sized to the class's
    /// `size_of_member_variables`. Install a host via
    /// `instance.vm.host = Some(Box::new(my_host))` before calling
    /// functions that use native calls.
    pub fn create_instance(&self, class_name: &str) -> Result<ScriptInstance, ScriptError> {
        let class_idx = self
            .find_class(class_name)
            .ok_or_else(|| ScriptError::ClassNotFound(class_name.to_owned()))?;
        Ok(self.create_instance_idx(class_idx))
    }

    /// Create an instance by class index. Panics if out of range.
    pub fn create_instance_idx(&self, class_idx: usize) -> ScriptInstance {
        let class = &self.program.scb.classes[class_idx];
        let heap_size = class.size_of_member_variables.max(0) as usize;

        let mut vm = Vm::new();
        vm.heap = vec![0u8; heap_size];

        ScriptInstance { class_idx, vm }
    }

    /// Tear down all loaded data.
    ///
    /// Only the mutable side (static area) is torn down here — the
    /// underlying `ScriptProgram` is an `Arc` and will be dropped when
    /// the last manager referencing it is dropped.
    pub fn destroy(&mut self) {
        self.program = std::sync::Arc::new(ScriptProgram::default());
        self.static_area.fill(0);
    }
}

// ───────────────────────── ScriptInstance ─────────────────────────

/// A VM instance bound to a specific script class.
///
/// Each game element (actor, zone, scroll, waypoint, etc.) gets its own
/// `ScriptInstance` with its own heap. The heap stores the class's member
/// variables — each instance has independent state.
///
/// # Usage
///
/// ```ignore
/// let mut mgr = ScriptManager::load_file("level.scb")?;
/// let mut inst = mgr.create_instance("StartUp")?;
/// inst.vm.host = Some(Box::new(ScriptEffects::new()));
/// let result = inst.call_function(&mut mgr, "Initialize")?;
/// ```
#[derive(Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash)]
pub struct ScriptInstance {
    /// Index into the ScriptManager's class/program arrays.
    class_idx: usize,
    /// The underlying VM. Caller sets `vm.host` before calling functions.
    pub vm: Vm,
}

impl fmt::Debug for ScriptInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScriptInstance")
            .field("class_idx", &self.class_idx)
            .field("ip", &self.vm.ip)
            .field("heap_len", &self.vm.heap.len())
            .field("frames", &self.vm.frames.len())
            .finish()
    }
}

impl ScriptInstance {
    /// The class index this instance is bound to.
    pub fn class_idx(&self) -> usize {
        self.class_idx
    }

    /// Check whether a function exists in this class.
    pub fn has_function(&self, manager: &ScriptManager, fn_name: &str) -> bool {
        self.find_function(manager, fn_name).is_some()
    }

    /// Look up a function by name.
    fn find_function<'a>(&self, manager: &'a ScriptManager, fn_name: &str) -> Option<&'a Function> {
        manager.program.scb.classes[self.class_idx]
            .functions
            .iter()
            .find(|f| f.name == fn_name)
    }

    /// List all function names in this class.
    pub fn function_names<'a>(&self, manager: &'a ScriptManager) -> Vec<&'a str> {
        manager.program.scb.classes[self.class_idx]
            .functions
            .iter()
            .map(|f| f.name.as_str())
            .collect()
    }

    /// Call a function by name.
    ///
    /// Use [`push_param`](Self::push_param) to stage parameters for
    /// the call. Scripts that use native calls should use
    /// [`call_function_with_host`](Self::call_function_with_host).
    ///
    /// The shared static area is synced from the manager before execution
    /// and written back after, so global variable changes propagate
    /// between instances.
    pub fn call_function(
        &mut self,
        manager: &mut ScriptManager,
        fn_name: &str,
    ) -> Result<i32, ScriptError> {
        let func = self
            .find_function(manager, fn_name)
            .ok_or_else(|| ScriptError::FunctionNotFound(fn_name.to_owned()))?;
        let entry_addr = func.address as u32;

        self.run_at(manager, entry_addr, fn_name)
    }

    /// Call a function by name with native calls dispatched through
    /// `host` for this execution only.
    pub fn call_function_with_host(
        &mut self,
        manager: &mut ScriptManager,
        fn_name: &str,
        host: &mut dyn HostFunctions,
    ) -> Result<i32, ScriptError> {
        let func = self
            .find_function(manager, fn_name)
            .ok_or_else(|| ScriptError::FunctionNotFound(fn_name.to_owned()))?;
        let entry_addr = func.address as u32;

        self.run_at_with_host(manager, entry_addr, fn_name, host)
    }

    /// Call a function by name with a custom step limit and native host.
    pub fn call_function_limited_with_host(
        &mut self,
        manager: &mut ScriptManager,
        fn_name: &str,
        max_steps: usize,
        host: &mut dyn HostFunctions,
    ) -> Result<i32, ScriptError> {
        let func = self
            .find_function(manager, fn_name)
            .ok_or_else(|| ScriptError::FunctionNotFound(fn_name.to_owned()))?;
        let entry_addr = func.address as u32;
        self.run_at_limited_with_host(manager, entry_addr, max_steps, fn_name, host)
    }

    /// Internal: set up the VM and run from the given entry address.
    fn run_at(
        &mut self,
        manager: &mut ScriptManager,
        entry_addr: u32,
        fn_name: &str,
    ) -> Result<i32, ScriptError> {
        self.run_at_limited(manager, entry_addr, 10_000_000, fn_name)
    }

    fn run_at_with_host(
        &mut self,
        manager: &mut ScriptManager,
        entry_addr: u32,
        fn_name: &str,
        host: &mut dyn HostFunctions,
    ) -> Result<i32, ScriptError> {
        self.run_at_limited_with_host(manager, entry_addr, 10_000_000, fn_name, host)
    }

    fn run_at_limited(
        &mut self,
        manager: &mut ScriptManager,
        entry_addr: u32,
        max_steps: usize,
        fn_name: &str,
    ) -> Result<i32, ScriptError> {
        self.begin_at(entry_addr);
        match self.resume_run(manager, max_steps, fn_name) {
            StopReason::ReturnedValue(v) => Ok(v),
            StopReason::Returned => Ok(0),
            other => Err(ScriptError::Vm(other)),
        }
    }

    fn run_at_limited_with_host(
        &mut self,
        manager: &mut ScriptManager,
        entry_addr: u32,
        max_steps: usize,
        fn_name: &str,
        host: &mut dyn HostFunctions,
    ) -> Result<i32, ScriptError> {
        self.begin_at(entry_addr);
        match self.resume_run_with_host(manager, max_steps, fn_name, host) {
            StopReason::ReturnedValue(v) => Ok(v),
            StopReason::Returned => Ok(0),
            other => Err(ScriptError::Vm(other)),
        }
    }

    /// Set up the VM frames + IP for a fresh function call without
    /// running anything yet.  Caller must have already pushed any
    /// parameters via [`push_param`](Self::push_param).
    ///
    /// Pairs with [`resume_run`](Self::resume_run) which actually
    /// drives the interpreter — splitting setup from drive lets
    /// callers handle [`StopReason::PendingNestedCall`] yields by
    /// dispatching the queued call out-of-band and re-invoking
    /// `resume_run` on the same instance, picking up at the IP the
    /// VM yielded at.
    pub fn begin_call(
        &mut self,
        manager: &ScriptManager,
        fn_name: &str,
    ) -> Result<(), ScriptError> {
        let func = self
            .find_function(manager, fn_name)
            .ok_or_else(|| ScriptError::FunctionNotFound(fn_name.to_owned()))?;
        let entry_addr = func.address as u32;
        self.begin_at(entry_addr);
        Ok(())
    }

    fn begin_at(&mut self, entry_addr: u32) {
        // Set up for a top-level call: fresh call stack with any staged
        // outgoing parameters as the bottom frame's incoming params.
        self.vm.frames.clear();
        let params = std::mem::take(&mut self.vm.outgoing_params);
        self.vm.frames.push(Frame {
            parameters: params,
            // return_address is u32::MAX — acts as a sentinel. If the
            // bottom frame's Return pops to this, the Vm's run loop
            // returns StopReason::Returned (no more frames).
            ..Default::default()
        });
        self.vm.ip = entry_addr;
    }

    /// Drive the VM forward from its current IP until it stops.  Syncs
    /// the shared static area in from `manager` before running and back
    /// out after, so both top-of-call setup and mid-call resume see the
    /// latest cross-instance global writes.
    ///
    /// Safe to call multiple times in a row to resume after a yield —
    /// no frame/IP reset between calls.  `fn_name` is only used for
    /// trace logging.
    pub fn resume_run(
        &mut self,
        manager: &mut ScriptManager,
        max_steps: usize,
        fn_name: &str,
    ) -> StopReason {
        // Sync shared static area into this instance's VM.  Called on
        // both the initial run and every resume — when a nested call
        // wrote to the static area mid-yield, that update needs to be
        // visible to this instance when it resumes.
        self.vm.static_area.resize(manager.static_area.len(), 0);
        self.vm.static_area.copy_from_slice(&manager.static_area);

        let class_name = &manager.program.scb.classes[self.class_idx].class_name;
        let program_len = manager.program.programs[self.class_idx].len();
        tracing::trace!(
            "resume_run {class_name}::{fn_name} starting (max_steps={max_steps}, program_len={program_len}, ip={})",
            self.vm.ip,
        );
        let start = web_time::Instant::now();
        let program = &manager.program.programs[self.class_idx];
        let stop = self.vm.run_up_to(program, max_steps);
        let elapsed = start.elapsed();
        tracing::trace!("resume_run {class_name}::{fn_name} done: {stop:?} ({elapsed:?})");

        // Sync static area back to the shared manager.
        let copy_len = manager.static_area.len().min(self.vm.static_area.len());
        manager.static_area[..copy_len].copy_from_slice(&self.vm.static_area[..copy_len]);
        stop
    }

    /// Drive the VM with native calls dispatched through `host`.
    pub fn resume_run_with_host(
        &mut self,
        manager: &mut ScriptManager,
        max_steps: usize,
        fn_name: &str,
        host: &mut dyn HostFunctions,
    ) -> StopReason {
        self.vm.static_area.resize(manager.static_area.len(), 0);
        self.vm.static_area.copy_from_slice(&manager.static_area);

        let class_name = &manager.program.scb.classes[self.class_idx].class_name;
        let program_len = manager.program.programs[self.class_idx].len();
        tracing::trace!(
            "resume_run {class_name}::{fn_name} starting (max_steps={max_steps}, program_len={program_len}, ip={})",
            self.vm.ip,
        );
        let start = web_time::Instant::now();
        let program = &manager.program.programs[self.class_idx];
        let stop = self.vm.run_up_to_with_host(program, max_steps, host);
        let elapsed = start.elapsed();
        tracing::trace!("resume_run {class_name}::{fn_name} done: {stop:?} ({elapsed:?})");

        let copy_len = manager.static_area.len().min(self.vm.static_area.len());
        manager.static_area[..copy_len].copy_from_slice(&self.vm.static_area[..copy_len]);
        stop
    }

    /// Push an i32 parameter for the next function call.
    pub fn push_param(&mut self, value: i32) {
        self.vm
            .outgoing_params
            .extend_from_slice(&value.to_le_bytes());
    }
}
