//! Script manager: loads `.scb` bytecode, manages class-bound VM instances.
//!
//! One `ScriptManager` per loaded `.scb` (i.e., per mission level). All
//! `ScriptInstance`s created from it share the same program code and static
//! memory area (the 0x0000..0x3FFF symbol range).

use std::fmt;

use crate::interp::{Frame, HostFunctions, StopReason, Vm};
use crate::scb::{Function, ScbFile};
use crate::vm::{self, Instruction};

// ───────────────────────── Errors ─────────────────────────

/// Errors from script manager operations.
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    /// Malformed bytecode; preparation never substitutes an instruction.
    #[error("invalid opcode {opcode:#04x} in class {class} at instruction {address}")]
    InvalidInstruction {
        class: String,
        address: usize,
        opcode: u8,
    },
    /// A decoded native call names no registered engine operation.
    #[error("unknown native {index} in class {class} at instruction {address}")]
    UnknownNative {
        class: String,
        address: usize,
        index: u32,
    },
    /// No class with this name exists in the loaded script.
    #[error("class not found: {0}")]
    ClassNotFound(String),
    /// No function with this name exists in the bound class.
    #[error("function not found: {0}")]
    FunctionNotFound(String),
    /// The VM stopped abnormally during execution.
    #[error("VM stopped abnormally: {0:?}")]
    Vm(StopReason),
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
    scb: ScbFile,
    /// Pre-decoded instruction streams, one per class, indexed in parallel
    /// with `scb.classes`.
    programs: Vec<Vec<Instruction>>,
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
    /// Read-only parsed source metadata corresponding to the decoded program.
    pub fn scb(&self) -> &ScbFile {
        &self.scb
    }

    /// Decode a parsed `.scb` file into a reusable `ScriptProgram`.
    pub fn from_scb(scb: ScbFile) -> Result<Self, ScriptError> {
        let programs = scb
            .classes
            .iter()
            .map(|class| {
                class
                    .quads
                    .iter()
                    .enumerate()
                    .map(|(address, q)| {
                        let instruction = vm::decode_for_preparation(*q).map_err(|_| {
                            ScriptError::InvalidInstruction {
                                class: class.class_name.clone(),
                                address,
                                opcode: q.operation,
                            }
                        })?;
                        if let Instruction::NativeCall { index } = instruction
                            && crate::natives::native_definition_by_index(index).is_none()
                        {
                            return Err(ScriptError::UnknownNative {
                                class: class.class_name.clone(),
                                address,
                                index,
                            });
                        }
                        Ok(instruction)
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { scb, programs })
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
    pub static_area: std::sync::Arc<Vec<u8>>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ScriptManagerSnapshot {
    static_area: Vec<u8>,
}

impl ScriptManagerSnapshot {
    pub(crate) fn capture(value: &ScriptManager) -> Self {
        let ScriptManager {
            program: _,
            static_area,
        } = value;
        Self {
            static_area: static_area.as_ref().clone(),
        }
    }

    pub(crate) fn into_runtime(self) -> ScriptManager {
        ScriptManager {
            program: std::sync::Arc::new(ScriptProgram::default()),
            static_area: self.static_area.into(),
        }
    }
}

impl crate::bitcode_adapters::NativeBitcode for ScriptManager {
    type Wire = ScriptManagerSnapshot;

    fn to_wire(&self) -> Self::Wire {
        ScriptManagerSnapshot::capture(self)
    }

    fn from_wire(snapshot: Self::Wire) -> Self {
        snapshot.into_runtime()
    }
}

crate::bitcode_adapters::impl_native_bitcode!(ScriptManager);

impl serde::Serialize for ScriptManager {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        ScriptManagerSnapshot::capture(self).serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for ScriptManager {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(ScriptManagerSnapshot::deserialize(deserializer)?.into_runtime())
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
        Self::from_program(std::sync::Arc::new(
            ScriptProgram::from_scb(scb).expect("ScriptManager requires validated SCB bytecode"),
        ))
    }

    /// Create a manager from host-owned immutable script bytecode.
    pub fn from_program(program: std::sync::Arc<ScriptProgram>) -> Self {
        Self {
            program,
            static_area: std::sync::Arc::new(vec![0u8; 4096]),
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
        std::sync::Arc::make_mut(&mut self.static_area).fill(0);
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
        self.vm.static_area = manager.static_area.clone();

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

        manager.static_area = self.vm.static_area.clone();
        stop
    }

    fn push_param(&mut self, value: i32) {
        self.vm
            .outgoing_params
            .extend_from_slice(&value.to_le_bytes());
    }
}

#[cfg(test)]
mod preparation_tests {
    use super::*;
    use crate::scb;

    fn program(opcodes: &[u8]) -> ScbFile {
        ScbFile {
            version: scb::SCB_VERSION,
            classes: vec![scb::ClassEntry {
                source_file: "probe.scs".into(),
                class_name: "Probe".into(),
                size_of_member_variables: 0,
                member_variables: vec![],
                functions: vec![],
                quads: opcodes
                    .iter()
                    .map(|operation| vm::Quad {
                        operation: *operation,
                        operands: [0; 8],
                    })
                    .collect(),
            }],
        }
    }

    #[test]
    fn malformed_opcode_reports_class_and_instruction_without_substitution() {
        let error = ScriptProgram::from_scb(program(&[0, 255])).unwrap_err();
        assert!(matches!(error, ScriptError::InvalidInstruction {
            ref class, address: 1, opcode: 255
        } if class == "Probe"));
    }

    #[test]
    fn invalid_native_is_rejected_before_it_can_corrupt_the_stack() {
        let mut scb = program(&[vm::Opcode::NativeCall as u8]);
        scb.classes[0].quads[0].operands[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            ScriptProgram::from_scb(scb),
            Err(ScriptError::UnknownNative {
                address: 0,
                index: u32::MAX,
                ..
            })
        ));
    }

    #[test]
    fn original_workaround_opcodes_remain_explicit_noops() {
        let program = ScriptProgram::from_scb(program(&[0, 58, 107, 208, 229])).unwrap();
        assert_eq!(program.programs[0], vec![Instruction::Empty; 5]);
        assert_eq!(
            program.scb().classes[0].quads.len(),
            program.programs[0].len()
        );
    }
}
