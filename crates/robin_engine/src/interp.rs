//! VM interpreter: executes decoded `Instruction`s against the
//! `Vm` memory state.
//!
//! Supports straight-line code, branches, arithmetic/comparison/move/
//! constant opcodes, script-to-script function calls (Call, Return,
//! ReturnVal, Param, GetParam, SetParam, GetReturn), and native
//! calls through a pluggable `HostFunctions` trait. Game integration
//! provides a HostFunctions impl that dispatches to the ~80 native
//! bindings.
//!
//! # Symbol encoding
//! Symbols are `u16`. The top two bits pick a memory region; the low
//! 14 bits are a byte offset within it:
//!
//! ```text
//!  0b00......   static area (shared across VM instances)
//!  0b01......   heap        (per-class-instance member storage)
//!  0b10......   volatile    (current activation record locals)
//!  0b11......   temporary   (current activation record scratch)
//! ```
//!
//! Primitives are 4-byte packed (int32 / float32). Writes pack little-
//! endian into the byte array; reads unpack the same way.

use crate::vm::{BinaryOp, Instruction, Symbol};

const REGION_STATIC: u16 = 0x0000;
const REGION_HEAP: u16 = 0x4000;
const REGION_VOLATILE: u16 = 0x8000;
const REGION_TEMP: u16 = 0xC000;

/// One activation-record's locals + incoming parameters + saved
/// return address and return value.
#[derive(
    Default, Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct Frame {
    /// Bytes received from the caller. Sized by the number of
    /// `Param` ops the caller emitted before `Call`.
    pub parameters: Vec<u8>,
    /// Locals. Sized by `BeginFunction`'s `volatile_count`.
    pub volatile: Vec<u8>,
    /// Compiler scratch. Sized by `BeginFunction`'s `temp_count`.
    pub temporary: Vec<u8>,
    /// Where to return to when this frame pops (IP after the caller's
    /// `Call`).
    pub return_address: u32,
    /// Populated by a callee's `ReturnVal` so the caller's `GetReturn`
    /// can read it.
    pub return_value: i32,
}

/// Execution result when the interpreter stops.
#[derive(Debug, Clone, PartialEq)]
pub enum StopReason {
    /// `Return` hit — top-of-stack return propagates out.
    Returned,
    /// `ReturnVal <sym>` hit with the named source — value copied out.
    ReturnedValue(i32),
    /// IP walked past end of the quad stream.
    RanOff,
    /// Hit an opcode this cut doesn't implement yet (Call, Param, …).
    Unimplemented(&'static str),
    /// `Empty` opcode — treated as fatal.
    HitEmpty,
    /// Hit the step-limit guard.
    StepLimit,
    /// A native called by `NativeCall` requested a nested-script call
    /// (see [`PendingNestedCall`]).  The interpreter has advanced the
    /// IP past the `NativeCall` instruction; the engine must dispatch
    /// the requested call, write its result into `vm.native_return_value`,
    /// and call `vm.run_up_to(...)` again to resume.  The script's
    /// next `Aff1NativeGetReturn` then reads the resolved value.
    PendingNestedCall(PendingNestedCall),
}

/// A nested-script call requested by a native (e.g. `PrototypeFilterEvent`)
/// during a running VM. Carried by value to the engine layer when the outer
/// VM yields with [`StopReason::PendingNestedCall`].
///
/// `fn_name` is owned `String` rather than `&'static str` so the same
/// type can carry both literal native-side dispatch names and (in the
/// future) script-supplied function names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingNestedCall {
    /// Actor script handle of the script instance to invoke against.
    pub actor_handle: i32,
    /// Function name to invoke on the target's bound script class.
    pub fn_name: String,
    /// i32 parameters to push onto the target VM before the call.
    pub params: Vec<i32>,
}

/// Result of one native dispatch.
///
/// A nested script call is control flow, not an integer return value. Keeping
/// it explicit prevents the VM from briefly staging a fake return and lets a
/// borrowed host hand the request directly back to the script driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeCallOutcome {
    Return(i32),
    PendingNestedCall(PendingNestedCall),
}

impl NativeCallOutcome {
    /// Extract an ordinary native return for call sites which cannot drive a
    /// nested script request (primarily focused tests and standalone tools).
    /// Failing loudly here prevents those adapters from reviving the old fake
    /// zero return for `PrototypeFilterEvent`.
    pub fn expect_return(self, context: &str) -> i32 {
        match self {
            Self::Return(value) => value,
            Self::PendingNestedCall(call) => {
                panic!("{context}: nested script call requires a MissionScript driver: {call:?}")
            }
        }
    }
}

/// Parameter stack for native calls. `NativeParam` pushes 4 bytes
/// each; the native function `pop_i32`s its own parameters in reverse
/// push order.
#[derive(
    Debug, Default, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct NativeStack {
    buffer: Vec<u8>,
}

impl NativeStack {
    pub fn len(&self) -> usize {
        self.buffer.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn push_i32(&mut self, v: i32) {
        self.buffer.extend_from_slice(&v.to_le_bytes());
    }

    /// Pop the top 4 bytes as an i32. Returns 0 if the stack is empty
    /// (the original engine doesn't bounds-check but would undershoot
    /// into garbage — 0 is the safe read).
    pub fn pop_i32(&mut self) -> i32 {
        let n = self.buffer.len();
        if n < 4 {
            return 0;
        }
        let v = i32::from_le_bytes([
            self.buffer[n - 4],
            self.buffer[n - 3],
            self.buffer[n - 2],
            self.buffer[n - 1],
        ]);
        self.buffer.truncate(n - 4);
        v
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

/// Host functions invoked by `NativeCall`. Each index corresponds to
/// an entry in the native-function registry. The host reads
/// parameters off the stack (pop them — the VM doesn't auto-clear)
/// and either returns a 32-bit value the script picks up with
/// `Aff1NativeGetReturn` or requests synchronous nested script dispatch.
pub trait HostFunctions {
    fn call(&mut self, index: u32, stack: &mut NativeStack) -> NativeCallOutcome;
}

impl<T: HostFunctions + ?Sized> HostFunctions for Box<T> {
    fn call(&mut self, index: u32, stack: &mut NativeStack) -> NativeCallOutcome {
        (**self).call(index, stack)
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash)]
pub struct Vm {
    /// Static area, shared across VM instances in the real engine. For
    /// single-function test harnesses we give each Vm its own.
    pub static_area: Vec<u8>,
    /// Class instance heap.
    pub heap: Vec<u8>,
    /// Call stack. The topmost frame is the one currently executing.
    /// Starts with a single bottom frame so top-level code has
    /// locals to work with.
    pub frames: Vec<Frame>,
    /// Staging area for outgoing `Param` values. `Call` transfers this
    /// into the new frame's `parameters` and resets this to empty.
    pub outgoing_params: Vec<u8>,
    /// Parameter stack for native calls.
    pub native_stack: NativeStack,
    /// Last native-call return value; read by Aff1NativeGetReturn.
    pub native_return_value: i32,
    /// Top-level return value (set when the bottom frame returns).
    pub return_value: i32,
    /// Instruction pointer (index into the quad stream).
    pub ip: u32,
}

impl Vm {
    pub fn new() -> Self {
        Self {
            static_area: vec![0; 4096],
            heap: vec![0; 4096],
            frames: vec![Frame::default()],
            outgoing_params: Vec::new(),
            native_stack: NativeStack::default(),
            native_return_value: 0,
            return_value: 0,
            ip: 0,
        }
    }

    /// Pair this VM with a native-call host for ad-hoc execution.
    /// The host lives in the returned wrapper, not in `Vm`, so the VM
    /// state itself stays fully serializable.
    pub fn with_host<H: HostFunctions>(self, host: H) -> VmWithHost<H> {
        VmWithHost { vm: self, host }
    }

    /// Mutable access to the current (top) frame. The Vm always has
    /// at least one frame, so this never panics.
    fn current_frame(&self) -> &Frame {
        self.frames.last().expect("frames never empty")
    }
    fn current_frame_mut(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("frames never empty")
    }

    /// Resolve a symbol to `(region_bytes, offset)`. Returns indices
    /// rather than a slice to avoid borrow-checker drama when the same
    /// instruction reads + writes the same region.
    fn region_offset(sym: Symbol) -> (u16, usize) {
        (sym & 0xC000, (sym & 0x3FFF) as usize)
    }

    fn bytes(&self, region: u16) -> &[u8] {
        match region {
            REGION_STATIC => &self.static_area,
            REGION_HEAP => &self.heap,
            REGION_VOLATILE => &self.current_frame().volatile,
            REGION_TEMP => &self.current_frame().temporary,
            _ => unreachable!("two high bits only have four values"),
        }
    }

    fn bytes_mut(&mut self, region: u16) -> &mut [u8] {
        match region {
            REGION_STATIC => &mut self.static_area,
            REGION_HEAP => &mut self.heap,
            REGION_VOLATILE => &mut self.current_frame_mut().volatile,
            REGION_TEMP => &mut self.current_frame_mut().temporary,
            _ => unreachable!(),
        }
    }

    fn read_i32(&self, sym: Symbol) -> i32 {
        let (region, off) = Self::region_offset(sym);
        let b = self.bytes(region);
        i32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
    }

    fn read_f32(&self, sym: Symbol) -> f32 {
        let (region, off) = Self::region_offset(sym);
        let b = self.bytes(region);
        f32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
    }

    fn write_i32(&mut self, sym: Symbol, v: i32) {
        let (region, off) = Self::region_offset(sym);
        let bytes = self.bytes_mut(region);
        bytes[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn write_f32(&mut self, sym: Symbol, v: f32) {
        let (region, off) = Self::region_offset(sym);
        let bytes = self.bytes_mut(region);
        bytes[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// Execute instructions starting at `self.ip` until a Return, Empty,
    /// Unimplemented, or end-of-stream is reached. Returns the stop
    /// reason. Hard-caps at 10M steps so runaway loops don't OOM.
    pub fn run(&mut self, program: &[Instruction]) -> StopReason {
        self.run_up_to(program, 10_000_000)
    }

    /// Execute with a native-call host supplied by the caller for this
    /// run only.
    pub fn run_with_host(
        &mut self,
        program: &[Instruction],
        host: &mut dyn HostFunctions,
    ) -> StopReason {
        self.run_up_to_with_host(program, 10_000_000, host)
    }

    /// Like `run` but with a caller-supplied step limit.
    pub fn run_up_to(&mut self, program: &[Instruction], max_steps: usize) -> StopReason {
        for _ in 0..max_steps {
            let Some(ins) = program.get(self.ip as usize) else {
                return StopReason::RanOff;
            };
            if let Some(stop) = self.step(*ins) {
                return stop;
            }
        }
        StopReason::StepLimit
    }

    /// Like `run_up_to` with a native-call host supplied by the caller
    /// for this run only.
    pub fn run_up_to_with_host(
        &mut self,
        program: &[Instruction],
        max_steps: usize,
        host: &mut dyn HostFunctions,
    ) -> StopReason {
        for _ in 0..max_steps {
            let Some(ins) = program.get(self.ip as usize) else {
                return StopReason::RanOff;
            };
            if let Some(stop) = self.step_with_host(*ins, Some(host)) {
                return stop;
            }
        }
        StopReason::StepLimit
    }

    /// Execute one instruction. Returns Some(stop) to halt, None to
    /// continue.
    pub fn step(&mut self, ins: Instruction) -> Option<StopReason> {
        self.step_with_host(ins, None)
    }

    /// Execute one instruction, optionally dispatching native calls to
    /// the supplied host.
    pub fn step_with_host(
        &mut self,
        ins: Instruction,
        host: Option<&mut dyn HostFunctions>,
    ) -> Option<StopReason> {
        use Instruction::*;
        match ins {
            Empty => return Some(StopReason::HitEmpty),
            Nop => self.ip += 1,
            EndFunction => {
                // EndFunction logs an error but still increments IP;
                // no shipped script should reach this.
                self.ip += 1;
            }
            Return => {
                if self.frames.len() == 1 {
                    // Bottom frame: control returns to the host.
                    return Some(StopReason::Returned);
                }
                let ret_addr = self.current_frame().return_address;
                self.frames.pop();
                self.ip = ret_addr;
            }
            ReturnVal { sym } => {
                let value = self.read_i32(sym);
                if self.frames.len() == 1 {
                    self.return_value = value;
                    return Some(StopReason::ReturnedValue(value));
                }
                // Write the return value onto the caller's frame
                // (the one below us) before popping.
                let caller_idx = self.frames.len() - 2;
                self.frames[caller_idx].return_value = value;
                self.return_value = value;
                let ret_addr = self.current_frame().return_address;
                self.frames.pop();
                self.ip = ret_addr;
            }

            // --- sizing of activation record ---
            BeginFunction {
                volatile_count,
                temp_count,
            } => {
                // The counts are "slots"; one slot is 4 bytes
                // (one int/float). We track as bytes here, so
                // multiply by 4.
                //
                // The frame must be zero-filled on resize: discard
                // the old buffer with `clear()` first so leading
                // bytes don't leak through.  Currently safe because
                // `BeginFunction` only ever runs on a fresh frame,
                // but we `clear()` to make the zero-on-resize
                // behaviour explicit and survive any future change
                // to that invariant.
                let frame = self.current_frame_mut();
                frame.volatile.clear();
                frame.volatile.resize(volatile_count as usize * 4, 0);
                frame.temporary.clear();
                frame.temporary.resize(temp_count as usize * 4, 0);
                self.ip += 1;
            }

            // --- control flow ---
            Goto { addr } => self.ip = addr,
            IfZeroGoto { sym, addr } => {
                if self.read_i32(sym) == 0 {
                    self.ip = addr
                } else {
                    self.ip += 1
                };
            }
            IfNotZeroGoto { sym, addr } => {
                if self.read_i32(sym) != 0 {
                    self.ip = addr
                } else {
                    self.ip += 1
                };
            }

            // --- moves / constants ---
            Aff0Integer { dst, src } => {
                let v = self.read_i32(src);
                self.write_i32(dst, v);
                self.ip += 1;
            }
            Aff0Float { dst, src } => {
                let v = self.read_f32(src);
                self.write_f32(dst, v);
                self.ip += 1;
            }
            Aff0IConstant { dst, constant } => {
                self.write_i32(dst, constant);
                self.ip += 1;
            }
            Aff0FConstant { dst, constant } => {
                self.write_f32(dst, constant);
                self.ip += 1;
            }

            // --- unary ---
            Aff1CastToInt { dst, src } => {
                let v = self.read_f32(src) as i32;
                self.write_i32(dst, v);
                self.ip += 1;
            }
            Aff1CastToFloat { dst, src } => {
                let v = self.read_i32(src) as f32;
                self.write_f32(dst, v);
                self.ip += 1;
            }
            Aff1IMinus { dst, src } => {
                let v = self.read_i32(src).wrapping_neg();
                self.write_i32(dst, v);
                self.ip += 1;
            }
            Aff1FMinus { dst, src } => {
                let v = -self.read_f32(src);
                self.write_f32(dst, v);
                self.ip += 1;
            }

            // --- binary ops ---
            Binary { op, dst, a, b } => self.step_binary(op, dst, a, b),

            // --- script-to-script calls ---
            Param { sym } => {
                let v = self.read_i32(sym);
                self.outgoing_params.extend_from_slice(&v.to_le_bytes());
                self.ip += 1;
            }
            Call { addr } => {
                // Transfer outgoing staging into the new frame's incoming
                // params; start a fresh empty staging for the callee's
                // own outgoing calls.
                let new_frame = Frame {
                    parameters: std::mem::take(&mut self.outgoing_params),
                    return_address: self.ip + 1,
                    ..Default::default()
                };
                self.frames.push(new_frame);
                self.ip = addr;
            }
            Aff1GetParam { dst, param_offset } => {
                let off = param_offset as usize;
                let bytes = &self.current_frame().parameters[off..off + 4];
                let v = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                self.write_i32(dst, v);
                self.ip += 1;
            }
            Aff1SetParam { dst_offset, src } => {
                let v = self.read_i32(src);
                let off = dst_offset as usize;
                self.current_frame_mut().parameters[off..off + 4].copy_from_slice(&v.to_le_bytes());
                self.ip += 1;
            }
            Aff1GetReturn { sym } => {
                let v = self.current_frame().return_value;
                self.write_i32(sym, v);
                self.ip += 1;
            }

            // --- native calls ---
            NativeParam { sym } => {
                let v = self.read_i32(sym);
                self.native_stack.push_i32(v);
                self.ip += 1;
            }
            NativeCall { index } => {
                let Some(host) = host else {
                    return Some(StopReason::Unimplemented("NativeCall"));
                };
                let outcome = host.call(index, &mut self.native_stack);
                // Always advance IP past the NativeCall before yielding —
                // when the engine resumes us after dispatching the
                // nested call, it will write the real i32 into
                // `native_return_value` and the next instruction
                // (typically `Aff1NativeGetReturn`) will read it.
                self.ip += 1;
                match outcome {
                    NativeCallOutcome::Return(value) => self.native_return_value = value,
                    NativeCallOutcome::PendingNestedCall(call) => {
                        return Some(StopReason::PendingNestedCall(call));
                    }
                }
            }
            Aff1NativeGetReturn { sym } => {
                let v = self.native_return_value;
                self.write_i32(sym, v);
                self.ip += 1;
            }
        }
        None
    }

    fn step_binary(&mut self, op: BinaryOp, dst: Symbol, a: Symbol, b: Symbol) {
        use BinaryOp::*;
        match op {
            // Integer arithmetic. Wrapping semantics
            // (no overflow checks on plain int).
            IAdd => {
                let r = self.read_i32(a).wrapping_add(self.read_i32(b));
                self.write_i32(dst, r);
            }
            ISub => {
                let r = self.read_i32(a).wrapping_sub(self.read_i32(b));
                self.write_i32(dst, r);
            }
            IMult => {
                let r = self.read_i32(a).wrapping_mul(self.read_i32(b));
                self.write_i32(dst, r);
            }
            IDiv => {
                let divisor = self.read_i32(b);
                assert!(divisor != 0, "VM IDiv attempted division by zero");
                // Trap on invalid integer division; `wrapping_div`
                // covers MIN / -1.
                let r = self.read_i32(a).wrapping_div(divisor);
                self.write_i32(dst, r);
            }
            IAor => {
                let r = self.read_i32(a) | self.read_i32(b);
                self.write_i32(dst, r);
            }
            IAand => {
                let r = self.read_i32(a) & self.read_i32(b);
                self.write_i32(dst, r);
            }
            IAxor => {
                let r = self.read_i32(a) ^ self.read_i32(b);
                self.write_i32(dst, r);
            }

            // Float arithmetic.
            FAdd => {
                let r = self.read_f32(a) + self.read_f32(b);
                self.write_f32(dst, r);
            }
            FSub => {
                let r = self.read_f32(a) - self.read_f32(b);
                self.write_f32(dst, r);
            }
            FMult => {
                let r = self.read_f32(a) * self.read_f32(b);
                self.write_f32(dst, r);
            }
            FDiv => {
                let r = self.read_f32(a) / self.read_f32(b);
                self.write_f32(dst, r);
            }

            // Integer comparisons. Result is 0/1 stored as i32.
            IInfEq => self.write_i32(dst, (self.read_i32(a) <= self.read_i32(b)) as i32),
            IInf => self.write_i32(dst, (self.read_i32(a) < self.read_i32(b)) as i32),
            ISupEq => self.write_i32(dst, (self.read_i32(a) >= self.read_i32(b)) as i32),
            ISup => self.write_i32(dst, (self.read_i32(a) > self.read_i32(b)) as i32),
            INeq => self.write_i32(dst, (self.read_i32(a) != self.read_i32(b)) as i32),
            IEq => self.write_i32(dst, (self.read_i32(a) == self.read_i32(b)) as i32),

            // Float comparisons. The bool result is written into a
            // float slot, so the destination holds 1.0f / 0.0f
            // (not 1 / 0 as int). Rust PartialOrd matches IEEE
            // semantics for the non-NaN case, which is all we need.
            FInfEq => {
                let r = if self.read_f32(a) <= self.read_f32(b) {
                    1.0
                } else {
                    0.0
                };
                self.write_f32(dst, r);
            }
            FInf => {
                let r = if self.read_f32(a) < self.read_f32(b) {
                    1.0
                } else {
                    0.0
                };
                self.write_f32(dst, r);
            }
            FSupEq => {
                let r = if self.read_f32(a) >= self.read_f32(b) {
                    1.0
                } else {
                    0.0
                };
                self.write_f32(dst, r);
            }
            FSup => {
                let r = if self.read_f32(a) > self.read_f32(b) {
                    1.0
                } else {
                    0.0
                };
                self.write_f32(dst, r);
            }
            FNeq => {
                let r = if self.read_f32(a) != self.read_f32(b) {
                    1.0
                } else {
                    0.0
                };
                self.write_f32(dst, r);
            }
            FEq => {
                let r = if self.read_f32(a) == self.read_f32(b) {
                    1.0
                } else {
                    0.0
                };
                self.write_f32(dst, r);
            }
        }
        self.ip += 1;
    }
}

/// Convenience wrapper for tests and standalone native-call execution.
pub struct VmWithHost<H> {
    pub vm: Vm,
    pub host: H,
}

impl<H: HostFunctions> VmWithHost<H> {
    pub fn run(&mut self, program: &[Instruction]) -> StopReason {
        self.vm.run_with_host(program, &mut self.host)
    }

    pub fn run_up_to(&mut self, program: &[Instruction], max_steps: usize) -> StopReason {
        self.vm
            .run_up_to_with_host(program, max_steps, &mut self.host)
    }

    pub fn take_host(self) -> H {
        self.host
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::{BinaryOp, Instruction::*};

    fn tmp(off: u16) -> u16 {
        REGION_TEMP | off
    }
    fn vol(off: u16) -> u16 {
        REGION_VOLATILE | off
    }
    fn heap(off: u16) -> u16 {
        REGION_HEAP | off
    }

    /// Build a Vm sized for a program that uses `slots` temp slots.
    fn vm_with_temp(slots: u16) -> Vm {
        let mut v = Vm::new();
        v.frames[0].temporary.resize(slots as usize * 4, 0);
        v
    }

    #[test]
    fn add_two_constants_and_return() {
        let program = vec![
            Aff0IConstant {
                dst: tmp(0),
                constant: 5,
            },
            Aff0IConstant {
                dst: tmp(4),
                constant: 7,
            },
            Binary {
                op: BinaryOp::IAdd,
                dst: tmp(8),
                a: tmp(0),
                b: tmp(4),
            },
            ReturnVal { sym: tmp(8) },
        ];
        let mut vm = vm_with_temp(3);
        assert_eq!(vm.run(&program), StopReason::ReturnedValue(12));
    }

    #[test]
    #[should_panic(expected = "VM IDiv attempted division by zero")]
    fn integer_division_by_zero_traps() {
        let program = vec![
            Aff0IConstant {
                dst: tmp(0),
                constant: 5,
            },
            Aff0IConstant {
                dst: tmp(4),
                constant: 0,
            },
            Binary {
                op: BinaryOp::IDiv,
                dst: tmp(8),
                a: tmp(0),
                b: tmp(4),
            },
        ];
        let mut vm = vm_with_temp(3);
        vm.run(&program);
    }

    #[test]
    fn float_arithmetic_and_compare() {
        // tmp[0] = 3.5; tmp[4] = 2.5; tmp[8] = tmp[0] * tmp[4]; tmp[12] = tmp[8] > tmp[0]; return tmp[12]
        let program = vec![
            Aff0FConstant {
                dst: tmp(0),
                constant: 3.5,
            },
            Aff0FConstant {
                dst: tmp(4),
                constant: 2.5,
            },
            Binary {
                op: BinaryOp::FMult,
                dst: tmp(8),
                a: tmp(0),
                b: tmp(4),
            },
            Binary {
                op: BinaryOp::FSup,
                dst: tmp(12),
                a: tmp(8),
                b: tmp(0),
            },
            ReturnVal { sym: tmp(12) },
        ];
        let mut vm = vm_with_temp(4);
        // FSup stores 1.0f / 0.0f via write_f32, so the returned
        // slot holds the bit pattern of 1.0f.
        assert_eq!(
            vm.run(&program),
            StopReason::ReturnedValue(1.0f32.to_bits() as i32)
        );
    }

    #[test]
    fn branching() {
        // if tmp[0] == 0: tmp[4] = 100 else tmp[4] = 200; return tmp[4]
        // tmp[0] is 0 by default.
        let program = vec![
            // IP 0: if tmp[0] != 0 goto 3
            IfNotZeroGoto {
                sym: tmp(0),
                addr: 3,
            },
            // IP 1: tmp[4] = 100
            Aff0IConstant {
                dst: tmp(4),
                constant: 100,
            },
            // IP 2: goto 4
            Goto { addr: 4 },
            // IP 3: tmp[4] = 200
            Aff0IConstant {
                dst: tmp(4),
                constant: 200,
            },
            // IP 4: return tmp[4]
            ReturnVal { sym: tmp(4) },
        ];
        let mut vm = vm_with_temp(2);
        assert_eq!(vm.run(&program), StopReason::ReturnedValue(100));
    }

    #[test]
    fn countdown_loop() {
        // i = 5; total = 0; while (i > 0) { total += i; i -= 1; } return total
        // tmp[0]=i, tmp[4]=total, tmp[8]=one, tmp[12]=cond
        let one = tmp(8);
        let cond = tmp(12);
        let i = tmp(0);
        let total = tmp(4);
        let program = vec![
            /* 0 */
            Aff0IConstant {
                dst: i,
                constant: 5,
            },
            /* 1 */
            Aff0IConstant {
                dst: total,
                constant: 0,
            },
            /* 2 */
            Aff0IConstant {
                dst: one,
                constant: 1,
            },
            // loop head
            /* 3 */
            Binary {
                op: BinaryOp::ISup,
                dst: cond,
                a: i,
                b: tmp(16),
            }, // cond = i > 0 (tmp[16]=0)
            /* 4 */ IfZeroGoto { sym: cond, addr: 8 },
            /* 5 */
            Binary {
                op: BinaryOp::IAdd,
                dst: total,
                a: total,
                b: i,
            },
            /* 6 */
            Binary {
                op: BinaryOp::ISub,
                dst: i,
                a: i,
                b: one,
            },
            /* 7 */ Goto { addr: 3 },
            /* 8 */ ReturnVal { sym: total },
        ];
        let mut vm = vm_with_temp(5); // tmp[0..16] in 4-byte slots = 5 slots
        assert_eq!(
            vm.run(&program),
            StopReason::ReturnedValue(5 + 4 + 3 + 2 + 1)
        );
    }

    #[test]
    fn heap_and_volatile_regions_work() {
        let mut vm = Vm::new();
        vm.heap.resize(64, 0);
        vm.frames[0].volatile.resize(64, 0);
        vm.frames[0].temporary.resize(64, 0);
        // Put values in each region, copy between them, verify.
        let program = vec![
            Aff0IConstant {
                dst: heap(0),
                constant: 0x1111_1111u32 as i32,
            },
            Aff0IConstant {
                dst: vol(0),
                constant: 0x2222_2222u32 as i32,
            },
            Aff0Integer {
                dst: tmp(0),
                src: heap(0),
            },
            Aff0Integer {
                dst: tmp(4),
                src: vol(0),
            },
            Binary {
                op: BinaryOp::IAdd,
                dst: tmp(8),
                a: tmp(0),
                b: tmp(4),
            },
            ReturnVal { sym: tmp(8) },
        ];
        assert_eq!(
            vm.run(&program),
            StopReason::ReturnedValue(0x3333_3333u32 as i32)
        );
    }

    #[test]
    fn stops_on_empty() {
        let mut vm = vm_with_temp(1);
        let program = vec![Empty];
        assert_eq!(vm.run(&program), StopReason::HitEmpty);
    }

    #[test]
    fn stops_on_unimplemented() {
        let mut vm = vm_with_temp(1);
        // NativeCall is still unimplemented (needs a host-fn registry).
        let program = vec![NativeCall { index: 0 }];
        assert_eq!(vm.run(&program), StopReason::Unimplemented("NativeCall"));
    }

    #[test]
    fn runs_off_end() {
        let mut vm = vm_with_temp(1);
        let program = vec![Nop, Nop];
        assert_eq!(vm.run(&program), StopReason::RanOff);
    }

    #[test]
    fn begin_function_sizes_frame() {
        let mut vm = Vm::new();
        let program = vec![
            BeginFunction {
                volatile_count: 2,
                temp_count: 4,
            },
            Aff0IConstant {
                dst: tmp(12),
                constant: 99,
            },
            ReturnVal { sym: tmp(12) },
        ];
        assert_eq!(vm.run(&program), StopReason::ReturnedValue(99));
        // BeginFunction sized temp to 4 slots = 16 bytes.
        assert_eq!(vm.frames[0].temporary.len(), 16);
        assert_eq!(vm.frames[0].volatile.len(), 8);
    }

    /// Classic "call a function that returns 42" end-to-end test.
    /// Layout:
    ///   @0 BEGIN_FN vol=0 tmp=1     ; outer
    ///   @1 CALL @4                   ; call inner
    ///   @2 GETRET -> tmp[0]          ; grab the return value
    ///   @3 RETURNVAL tmp[0]
    ///   @4 BEGIN_FN vol=0 tmp=1     ; inner
    ///   @5 IMOV_K tmp[0] = 42
    ///   @6 RETURNVAL tmp[0]
    #[test]
    fn call_into_function_returns_value() {
        let program = vec![
            BeginFunction {
                volatile_count: 0,
                temp_count: 1,
            }, // @0
            Call { addr: 4 },              // @1
            Aff1GetReturn { sym: tmp(0) }, // @2
            ReturnVal { sym: tmp(0) },     // @3
            BeginFunction {
                volatile_count: 0,
                temp_count: 1,
            }, // @4
            Aff0IConstant {
                dst: tmp(0),
                constant: 42,
            }, // @5
            ReturnVal { sym: tmp(0) },     // @6
        ];
        let mut vm = Vm::new();
        assert_eq!(vm.run(&program), StopReason::ReturnedValue(42));
    }

    /// Pass a parameter into a callee, use it, return the result.
    ///   outer: PARAM src=tmp[0](7); CALL inner; GETRET tmp[1]; RETURNVAL tmp[1]
    ///   inner: BEGIN_FN tmp=1; GETPARAM tmp[0] <- param[0]; add 1; return
    #[test]
    fn param_roundtrip_through_callee() {
        let program = vec![
            /* 0 */
            BeginFunction {
                volatile_count: 0,
                temp_count: 2,
            },
            /* 1 */
            Aff0IConstant {
                dst: tmp(0),
                constant: 7,
            },
            /* 2 */ Param { sym: tmp(0) },
            /* 3 */ Call { addr: 6 },
            /* 4 */ Aff1GetReturn { sym: tmp(4) },
            /* 5 */ ReturnVal { sym: tmp(4) },
            /* 6 */
            BeginFunction {
                volatile_count: 0,
                temp_count: 2,
            },
            /* 7 */
            Aff1GetParam {
                dst: tmp(0),
                param_offset: 0,
            },
            /* 8 */
            Aff0IConstant {
                dst: tmp(4),
                constant: 1,
            },
            /* 9 */
            Binary {
                op: BinaryOp::IAdd,
                dst: tmp(0),
                a: tmp(0),
                b: tmp(4),
            },
            /* 10 */ ReturnVal { sym: tmp(0) },
        ];
        let mut vm = Vm::new();
        assert_eq!(vm.run(&program), StopReason::ReturnedValue(8));
    }

    /// Recursive factorial — exercises the call stack and return
    /// values. fact(n) = n * fact(n-1), fact(0) = 1.
    #[test]
    fn recursive_factorial() {
        // tmp[0] = n, tmp[4] = zero, tmp[8] = is_zero, tmp[12] = one,
        // tmp[16] = n-1, tmp[20] = recursive result, tmp[24] = product
        let program = vec![
            // outer: fact(5)
            /*  0 */
            BeginFunction {
                volatile_count: 0,
                temp_count: 1,
            },
            /*  1 */
            Aff0IConstant {
                dst: tmp(0),
                constant: 5,
            },
            /*  2 */ Param { sym: tmp(0) },
            /*  3 */ Call { addr: 6 },
            /*  4 */ Aff1GetReturn { sym: tmp(0) },
            /*  5 */ ReturnVal { sym: tmp(0) },
            // fact(n):
            /*  6 */
            BeginFunction {
                volatile_count: 0,
                temp_count: 7,
            },
            /*  7 */
            Aff1GetParam {
                dst: tmp(0),
                param_offset: 0,
            },
            /*  8 */
            Aff0IConstant {
                dst: tmp(4),
                constant: 0,
            },
            /*  9 */
            Binary {
                op: BinaryOp::IEq,
                dst: tmp(8),
                a: tmp(0),
                b: tmp(4),
            },
            /* 10 */
            IfZeroGoto {
                sym: tmp(8),
                addr: 13,
            }, // if !zero, recurse
            /* 11 */
            Aff0IConstant {
                dst: tmp(0),
                constant: 1,
            }, // base case
            /* 12 */ ReturnVal { sym: tmp(0) },
            /* 13 */
            Aff0IConstant {
                dst: tmp(12),
                constant: 1,
            },
            /* 14 */
            Binary {
                op: BinaryOp::ISub,
                dst: tmp(16),
                a: tmp(0),
                b: tmp(12),
            },
            /* 15 */ Param { sym: tmp(16) },
            /* 16 */ Call { addr: 6 },
            /* 17 */ Aff1GetReturn { sym: tmp(20) },
            /* 18 */
            Binary {
                op: BinaryOp::IMult,
                dst: tmp(24),
                a: tmp(0),
                b: tmp(20),
            },
            /* 19 */ ReturnVal { sym: tmp(24) },
        ];
        let mut vm = Vm::new();
        assert_eq!(vm.run(&program), StopReason::ReturnedValue(120));
    }

    /// Plain `Return` (no value) unwinds a frame without touching
    /// the caller's return_value.
    #[test]
    fn plain_return_pops_frame() {
        let program = vec![
            /* 0 */
            BeginFunction {
                volatile_count: 0,
                temp_count: 1,
            },
            /* 1 */
            Aff0IConstant {
                dst: tmp(0),
                constant: 99,
            }, // pre-populate
            /* 2 */ Call { addr: 5 },
            /* 3 */ Aff1GetReturn { sym: tmp(0) },
            /* 4 */ ReturnVal { sym: tmp(0) },
            /* 5 */
            BeginFunction {
                volatile_count: 0,
                temp_count: 0,
            },
            /* 6 */ Return,
        ];
        let mut vm = Vm::new();
        // The callee returned with no value; GetReturn reads the
        // caller frame's return_value, which defaults to 0.
        assert_eq!(vm.run(&program), StopReason::ReturnedValue(0));
    }

    /// Minimal host implementation for tests: two fns, each named
    /// by index. fn 0 adds its two params, fn 1 multiplies them.
    #[derive(Clone)]
    struct TestHost;
    impl HostFunctions for TestHost {
        fn call(&mut self, index: u32, stack: &mut NativeStack) -> NativeCallOutcome {
            let b = stack.pop_i32();
            let a = stack.pop_i32();
            NativeCallOutcome::Return(match index {
                0 => a + b,
                1 => a * b,
                _ => 0,
            })
        }
    }

    #[derive(Clone)]
    struct NestedCallHost;

    impl HostFunctions for NestedCallHost {
        fn call(&mut self, _index: u32, _stack: &mut NativeStack) -> NativeCallOutcome {
            NativeCallOutcome::PendingNestedCall(PendingNestedCall {
                actor_handle: 23,
                fn_name: "FilterAIEvent".to_owned(),
                params: vec![7, 11],
            })
        }
    }

    #[test]
    fn nested_native_yields_after_advancing_ip_and_resumes_with_resolved_value() {
        let program = [
            BeginFunction {
                volatile_count: 0,
                temp_count: 1,
            },
            NativeCall { index: 108 },
            Aff1NativeGetReturn { sym: tmp(0) },
            ReturnVal { sym: tmp(0) },
        ];
        let mut vm = Vm::new();
        let mut host = NestedCallHost;

        let stop = vm.run_up_to_with_host(&program, 10, &mut host);
        assert_eq!(vm.ip, 2, "resume must start after NativeCall");
        assert_eq!(
            vm.native_return_value, 0,
            "a nested call must not stage a fake native return"
        );

        let StopReason::PendingNestedCall(pending) = stop else {
            panic!("expected nested-call yield, got {stop:?}");
        };
        assert_eq!(pending.actor_handle, 23);
        assert_eq!(pending.fn_name, "FilterAIEvent");
        assert_eq!(pending.params, [7, 11]);

        vm.native_return_value = 42;
        assert_eq!(
            vm.run_up_to_with_host(&program, 10, &mut host),
            StopReason::ReturnedValue(42),
            "Aff1NativeGetReturn must observe the resolved nested result"
        );
    }

    struct BorrowingHost<'a> {
        total: &'a mut i32,
    }

    impl HostFunctions for BorrowingHost<'_> {
        fn call(&mut self, _index: u32, stack: &mut NativeStack) -> NativeCallOutcome {
            *self.total += stack.pop_i32();
            NativeCallOutcome::Return(*self.total)
        }
    }

    #[test]
    fn vm_with_host_accepts_a_non_static_borrowing_dispatcher() {
        let program = [
            BeginFunction {
                volatile_count: 0,
                temp_count: 2,
            },
            Aff0IConstant {
                dst: tmp(0),
                constant: 7,
            },
            NativeParam { sym: tmp(0) },
            NativeCall { index: 0 },
            Aff1NativeGetReturn { sym: tmp(4) },
            ReturnVal { sym: tmp(4) },
        ];
        let mut total = 5;

        let stop = {
            let host = BorrowingHost { total: &mut total };
            let mut vm = Vm::new().with_host(host);
            vm.run(&program)
        };

        assert_eq!(stop, StopReason::ReturnedValue(12));
        assert_eq!(total, 12);
    }

    #[test]
    fn native_call_adds() {
        // tmp[0] = 3; tmp[4] = 4; native_call 0 (add); return result
        let program = vec![
            Aff0IConstant {
                dst: tmp(0),
                constant: 3,
            },
            Aff0IConstant {
                dst: tmp(4),
                constant: 4,
            },
            NativeParam { sym: tmp(0) },
            NativeParam { sym: tmp(4) },
            NativeCall { index: 0 },
            Aff1NativeGetReturn { sym: tmp(8) },
            ReturnVal { sym: tmp(8) },
        ];
        let mut vm = vm_with_temp(3);
        let mut host = TestHost;
        assert_eq!(
            vm.run_with_host(&program, &mut host),
            StopReason::ReturnedValue(7)
        );
    }

    #[test]
    fn native_call_without_host_stops() {
        let program = vec![
            NativeParam { sym: tmp(0) },
            NativeCall { index: 0 },
            Aff1NativeGetReturn { sym: tmp(0) },
        ];
        let mut vm = vm_with_temp(1);
        assert_eq!(vm.run(&program), StopReason::Unimplemented("NativeCall"));
    }

    #[test]
    fn native_stack_order_is_reverse_push() {
        // Push 10, 20, 30; pop -> 30, 20, 10.
        let mut s = NativeStack::default();
        s.push_i32(10);
        s.push_i32(20);
        s.push_i32(30);
        assert_eq!(s.pop_i32(), 30);
        assert_eq!(s.pop_i32(), 20);
        assert_eq!(s.pop_i32(), 10);
        assert_eq!(s.pop_i32(), 0); // underflow returns 0
        assert!(s.is_empty());
    }

    /// Outgoing params staging resets per call: consecutive Calls
    /// don't leak params from one to the next.
    #[test]
    fn outgoing_params_isolated_per_call() {
        // Call f(3), then call g(5). Both should see their own param.
        // f returns its param + 100, g returns its param + 200.
        // Outer adds them: (3+100) + (5+200) = 308.
        let program = vec![
            /*  0 */
            BeginFunction {
                volatile_count: 0,
                temp_count: 3,
            },
            /*  1 */
            Aff0IConstant {
                dst: tmp(0),
                constant: 3,
            },
            /*  2 */ Param { sym: tmp(0) },
            /*  3 */ Call { addr: 13 }, // f
            /*  4 */ Aff1GetReturn { sym: tmp(0) },
            /*  5 */
            Aff0IConstant {
                dst: tmp(4),
                constant: 5,
            },
            /*  6 */ Param { sym: tmp(4) },
            /*  7 */ Call { addr: 18 }, // g
            /*  8 */ Aff1GetReturn { sym: tmp(4) },
            /*  9 */
            Binary {
                op: BinaryOp::IAdd,
                dst: tmp(8),
                a: tmp(0),
                b: tmp(4),
            },
            /* 10 */ ReturnVal { sym: tmp(8) },
            /* 11 */ Nop,
            /* 12 */ Nop,
            // f(x) = x + 100
            /* 13 */
            BeginFunction {
                volatile_count: 0,
                temp_count: 2,
            },
            /* 14 */
            Aff1GetParam {
                dst: tmp(0),
                param_offset: 0,
            },
            /* 15 */
            Aff0IConstant {
                dst: tmp(4),
                constant: 100,
            },
            /* 16 */
            Binary {
                op: BinaryOp::IAdd,
                dst: tmp(0),
                a: tmp(0),
                b: tmp(4),
            },
            /* 17 */ ReturnVal { sym: tmp(0) },
            // g(x) = x + 200
            /* 18 */
            BeginFunction {
                volatile_count: 0,
                temp_count: 2,
            },
            /* 19 */
            Aff1GetParam {
                dst: tmp(0),
                param_offset: 0,
            },
            /* 20 */
            Aff0IConstant {
                dst: tmp(4),
                constant: 200,
            },
            /* 21 */
            Binary {
                op: BinaryOp::IAdd,
                dst: tmp(0),
                a: tmp(0),
                b: tmp(4),
            },
            /* 22 */ ReturnVal { sym: tmp(0) },
        ];
        let mut vm = Vm::new();
        assert_eq!(vm.run(&program), StopReason::ReturnedValue(308));
    }
}
