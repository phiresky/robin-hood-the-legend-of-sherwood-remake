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
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
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

#[derive(serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ScriptManagerSnapshot {
    static_area: Vec<u8>,
}

impl crate::bitcode_adapters::NativeBitcode for ScriptManager {
    type Wire = ScriptManagerSnapshot;

    fn to_wire(&self) -> Self::Wire {
        ScriptManagerSnapshot {
            static_area: self.static_area.clone(),
        }
    }

    fn from_wire(snapshot: Self::Wire) -> Self {
        Self {
            program: std::sync::Arc::new(ScriptProgram::default()),
            static_area: snapshot.static_area,
        }
    }
}

crate::bitcode_adapters::impl_native_bitcode!(ScriptManager);

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
    /// `size_of_member_variables`. Engine execution uses the explicit
    /// activation/polling API so synchronous native yields cannot be bypassed.
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
#[derive(
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
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
    /// Create an independent activation for a top-level callback without
    /// leaving its frames installed on the persistent instance.
    pub fn begin_activation(
        &mut self,
        manager: &ScriptManager,
        fn_name: &str,
        params: &[i32],
    ) -> Result<crate::interp::VmActivationState, ScriptError> {
        for &param in params {
            self.push_param(param);
        }
        self.begin_call(manager, fn_name)?;
        Ok(self.vm.take_activation())
    }

    /// Poll one activation against this instance's canonical heap.
    /// Activation state is restored even if native dispatch panics.
    pub fn poll_activation_with_host(
        &mut self,
        manager: &mut ScriptManager,
        activation: &mut crate::interp::VmActivationState,
        max_steps: usize,
        fn_name: &str,
        host: &mut dyn HostFunctions,
    ) -> StopReason {
        self.vm.swap_activation(activation);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.resume_run_with_host(manager, max_steps, fn_name, host)
        }));
        self.vm.swap_activation(activation);
        match result {
            Ok(stop) => stop,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

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

    /// Set up the VM frames + IP for a fresh activation. The public entry is
    /// [`begin_activation`](Self::begin_activation), which supplies parameters
    /// explicitly and pairs with the yield-aware polling API.
    fn begin_call(&mut self, manager: &ScriptManager, fn_name: &str) -> Result<(), ScriptError> {
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

    /// Drive the VM with native calls dispatched through `host`.
    fn resume_run_with_host(
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

    fn push_param(&mut self, value: i32) {
        self.vm
            .outgoing_params
            .extend_from_slice(&value.to_le_bytes());
    }
}
