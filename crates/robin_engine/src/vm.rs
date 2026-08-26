//! VM instruction decoding.
//!
//! A `Quad` as it comes off disk is an opaque `{u8 op, [u8;8] operands}`.
//! This module maps it to a typed `Instruction`. Which of the operand
//! union variants is in play depends on the opcode — the mapping comes
//! from the per-opcode byte-swap table the original serializer used,
//! plus the operand-struct layouts.
//!
//! Decoding is pure and cheap; callers are free to decode ahead of time
//! or lazily at dispatch. The interpreter doesn't live here — this is
//! just the front-end.

/// A single bytecode instruction. Opcode semantics aren't decoded here —
/// we keep the raw 1+8-byte layout so the interpreter can dispatch later
/// with no format surprises.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct Quad {
    pub operation: u8,
    pub operands: [u8; 8],
}

/// Raw opcode byte. The `u8` discriminants are what appears on disk in
/// `.scb` files.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, num_enum::TryFromPrimitive)]
pub enum Opcode {
    Empty = 0,
    Nop = 1,
    Param = 2,
    BeginFunction = 3,
    EndFunction = 4,
    Call = 5,
    Return = 6,
    ReturnVal = 7,
    Aff1GetParam = 8,
    Aff1SetParam = 9,
    Aff1GetReturn = 10,
    NativeParam = 11,
    NativeCall = 12,
    Aff1NativeGetReturn = 13,
    Goto = 14,
    IfNotZeroGoto = 15,
    IfZeroGoto = 16,
    Aff0Integer = 17,
    Aff0Float = 18,
    Aff0IConstant = 19,
    Aff0FConstant = 20,
    Aff1IMinus = 21,
    Aff1FMinus = 22,
    Aff1CastToInt = 23,
    Aff1CastToFloat = 24,
    Aff2IAdd = 25,
    Aff2ISub = 26,
    Aff2IMult = 27,
    Aff2IDiv = 28,
    Aff2IAor = 29,
    Aff2IAand = 30,
    Aff2IAxor = 31,
    Aff2FAdd = 32,
    Aff2FSub = 33,
    Aff2FMult = 34,
    Aff2FDiv = 35,
    Aff2IInfEq = 36,
    Aff2IInf = 37,
    Aff2ISupEq = 38,
    Aff2ISup = 39,
    Aff2INeq = 40,
    Aff2IEq = 41,
    Aff2FInfEq = 42,
    Aff2FInf = 43,
    Aff2FSupEq = 44,
    Aff2FSup = 45,
    Aff2FNeq = 46,
    Aff2FEq = 47,
}

impl Opcode {
    pub fn from_u8(b: u8) -> Option<Self> {
        Self::try_from(b).ok()
    }
}

/// A symbol table index — `u16` on disk.
pub type Symbol = u16;
/// A code address / quad index — `u32` on disk.
pub type Address = u32;

/// A decoded instruction. Each variant carries exactly the operand
/// layout the opcode expects, so the interpreter doesn't need to know
/// the union rules.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum Instruction {
    /// `Q_EMPTY` — an error marker; the interpreter treats it as fatal.
    Empty,
    /// `Q_NOP` — no-op, `ip += 1`.
    Nop,
    /// `Q_ENDFUNCTION` — marker at end of a function body; should be
    /// unreachable (functions Return first).
    EndFunction,
    /// `Q_RETURN` — return from current function with no value.
    Return,

    /// `Q_PARAM sym`: push local `sym` as a script parameter.
    Param { sym: Symbol },
    /// `Q_RETURNVAL sym`: return the value of local `sym`.
    ReturnVal { sym: Symbol },
    /// `Q_NATIVEPARAM sym`: push local `sym` onto the native-call stack.
    NativeParam { sym: Symbol },
    /// `Q_AFF1_NATIVEGETRETURN sym`: store the most-recent native
    /// return value into `sym`.
    Aff1NativeGetReturn { sym: Symbol },
    /// `Q_AFF1_GETRETURN sym`: store the most-recent script return
    /// value into `sym`.
    Aff1GetReturn { sym: Symbol },

    /// `Q_BEGINFUNCTION vol_count temp_count`: at function entry, size
    /// the volatile and temporary arrays in the activation record.
    BeginFunction {
        volatile_count: Symbol,
        temp_count: Symbol,
    },

    /// `Q_AFF1_CAST_TO_INT dst src` — truncate float `src` into int `dst`.
    Aff1CastToInt { dst: Symbol, src: Symbol },
    /// `Q_AFF1_CAST_TO_FLOAT dst src`.
    Aff1CastToFloat { dst: Symbol, src: Symbol },
    /// `Q_AFF1_IMINUS dst src` — integer negation.
    Aff1IMinus { dst: Symbol, src: Symbol },
    /// `Q_AFF1_FMINUS dst src` — float negation.
    Aff1FMinus { dst: Symbol, src: Symbol },
    /// `Q_AFF0_INTEGER dst src` — copy int.
    Aff0Integer { dst: Symbol, src: Symbol },
    /// `Q_AFF0_FLOAT dst src` — copy float.
    Aff0Float { dst: Symbol, src: Symbol },

    /// `Q_GOTO addr`.
    Goto { addr: Address },
    /// `Q_CALL addr` — call function at code address `addr`.
    Call { addr: Address },
    /// `Q_NATIVECALL index` — call native function by registry index.
    NativeCall { index: Address },

    /// `Q_AFF0_ICONSTANT dst const`.
    Aff0IConstant { dst: Symbol, constant: i32 },
    /// `Q_AFF1_GETPARAM dst param_offset` — fetch incoming param at
    /// byte offset `param_offset`.
    Aff1GetParam { dst: Symbol, param_offset: i32 },

    /// `Q_AFF1_SETPARAM dst_offset src` — place `src` at outgoing param
    /// byte offset `dst_offset`.
    Aff1SetParam { dst_offset: i32, src: Symbol },

    /// `Q_IF_ZERO_GOTO sym addr` — branch if `sym` == 0.
    IfZeroGoto { sym: Symbol, addr: Address },
    /// `Q_IF_NOT_ZERO_GOTO sym addr`.
    IfNotZeroGoto { sym: Symbol, addr: Address },

    /// `Q_AFF0_FCONSTANT dst const`.
    Aff0FConstant { dst: Symbol, constant: f32 },

    /// Three-symbol binary ops (`dst = a OP b`). The `op` field names
    /// the specific opcode so the interpreter can dispatch a single
    /// enum variant.
    Binary {
        op: BinaryOp,
        dst: Symbol,
        a: Symbol,
        b: Symbol,
    },
}

/// Tag for three-symbol binary ops.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum BinaryOp {
    IAdd,
    ISub,
    IMult,
    IDiv,
    IAor,
    IAand,
    IAxor,
    FAdd,
    FSub,
    FMult,
    FDiv,
    IInfEq,
    IInf,
    ISupEq,
    ISup,
    INeq,
    IEq,
    FInfEq,
    FInf,
    FSupEq,
    FSup,
    FNeq,
    FEq,
}

/// Decode failure — either the opcode byte is outside the known range
/// or (in the caller's wire format) the operand bytes are malformed.
/// Right now decoding can only fail at the opcode step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    UnknownOpcode(u8),
}

/// Decode a raw `Quad` into a typed `Instruction`.
pub fn decode(q: Quad) -> Result<Instruction, DecodeError> {
    let op = Opcode::from_u8(q.operation).ok_or(DecodeError::UnknownOpcode(q.operation))?;
    Ok(decode_with(op, q.operands))
}

fn decode_with(op: Opcode, ops: [u8; 8]) -> Instruction {
    // Little-endian decoders, all reading from offset 0 of the 8-byte
    // operand block.
    let u16_at = |off: usize| u16::from_le_bytes([ops[off], ops[off + 1]]);
    let u32_at =
        |off: usize| u32::from_le_bytes([ops[off], ops[off + 1], ops[off + 2], ops[off + 3]]);
    let i32_at =
        |off: usize| i32::from_le_bytes([ops[off], ops[off + 1], ops[off + 2], ops[off + 3]]);
    let f32_at =
        |off: usize| f32::from_le_bytes([ops[off], ops[off + 1], ops[off + 2], ops[off + 3]]);

    use BinaryOp::*;
    use Instruction::*;
    use Opcode as O;

    match op {
        O::Empty => Empty,
        O::Nop => Nop,
        O::EndFunction => EndFunction,
        O::Return => Return,

        // 1S: single symbol at offset 0.
        O::Param => Param { sym: u16_at(0) },
        O::ReturnVal => ReturnVal { sym: u16_at(0) },
        O::NativeParam => NativeParam { sym: u16_at(0) },
        O::Aff1NativeGetReturn => Aff1NativeGetReturn { sym: u16_at(0) },
        O::Aff1GetReturn => Aff1GetReturn { sym: u16_at(0) },

        // 1A: single address at offset 0.
        O::Goto => Goto { addr: u32_at(0) },
        O::Call => Call { addr: u32_at(0) },
        O::NativeCall => NativeCall { index: u32_at(0) },

        // 2SS: two symbols at offsets 0 and 2.
        O::BeginFunction => BeginFunction {
            volatile_count: u16_at(0),
            temp_count: u16_at(2),
        },
        O::Aff1CastToInt => Aff1CastToInt {
            dst: u16_at(0),
            src: u16_at(2),
        },
        O::Aff1CastToFloat => Aff1CastToFloat {
            dst: u16_at(0),
            src: u16_at(2),
        },
        O::Aff1IMinus => Aff1IMinus {
            dst: u16_at(0),
            src: u16_at(2),
        },
        O::Aff1FMinus => Aff1FMinus {
            dst: u16_at(0),
            src: u16_at(2),
        },
        O::Aff0Integer => Aff0Integer {
            dst: u16_at(0),
            src: u16_at(2),
        },
        O::Aff0Float => Aff0Float {
            dst: u16_at(0),
            src: u16_at(2),
        },

        // 2SI: sym at 0, int at 4.
        O::Aff0IConstant => Aff0IConstant {
            dst: u16_at(0),
            constant: i32_at(4),
        },
        O::Aff1GetParam => Aff1GetParam {
            dst: u16_at(0),
            param_offset: i32_at(4),
        },

        // 2IS: on the wire this is identical to 2SI — sym(u16)@0 then
        // int(i32)@4. The original `SetParam` flips the natural
        // interpretation: the u16-at-0 slot is the source symbol and
        // the i32-at-4 slot is the destination parameter offset. We
        // copy that meaning verbatim.
        O::Aff1SetParam => Aff1SetParam {
            dst_offset: i32_at(4),
            src: u16_at(0),
        },

        // 2SA: sym at 0, address at 4.
        O::IfZeroGoto => IfZeroGoto {
            sym: u16_at(0),
            addr: u32_at(4),
        },
        O::IfNotZeroGoto => IfNotZeroGoto {
            sym: u16_at(0),
            addr: u32_at(4),
        },

        // 2SR: sym at 0, float at 4.
        O::Aff0FConstant => Aff0FConstant {
            dst: u16_at(0),
            constant: f32_at(4),
        },

        // 3SSS: three symbols at offsets 0, 2, 4.
        O::Aff2IAdd
        | O::Aff2ISub
        | O::Aff2IMult
        | O::Aff2IDiv
        | O::Aff2IAor
        | O::Aff2IAand
        | O::Aff2IAxor
        | O::Aff2FAdd
        | O::Aff2FSub
        | O::Aff2FMult
        | O::Aff2FDiv
        | O::Aff2IInfEq
        | O::Aff2IInf
        | O::Aff2ISupEq
        | O::Aff2ISup
        | O::Aff2INeq
        | O::Aff2IEq
        | O::Aff2FInfEq
        | O::Aff2FInf
        | O::Aff2FSupEq
        | O::Aff2FSup
        | O::Aff2FNeq
        | O::Aff2FEq => {
            let bin_op = match op {
                O::Aff2IAdd => IAdd,
                O::Aff2ISub => ISub,
                O::Aff2IMult => IMult,
                O::Aff2IDiv => IDiv,
                O::Aff2IAor => IAor,
                O::Aff2IAand => IAand,
                O::Aff2IAxor => IAxor,
                O::Aff2FAdd => FAdd,
                O::Aff2FSub => FSub,
                O::Aff2FMult => FMult,
                O::Aff2FDiv => FDiv,
                O::Aff2IInfEq => IInfEq,
                O::Aff2IInf => IInf,
                O::Aff2ISupEq => ISupEq,
                O::Aff2ISup => ISup,
                O::Aff2INeq => INeq,
                O::Aff2IEq => IEq,
                O::Aff2FInfEq => FInfEq,
                O::Aff2FInf => FInf,
                O::Aff2FSupEq => FSupEq,
                O::Aff2FSup => FSup,
                O::Aff2FNeq => FNeq,
                O::Aff2FEq => FEq,
                _ => unreachable!(),
            };
            Binary {
                op: bin_op,
                dst: u16_at(0),
                a: u16_at(2),
                b: u16_at(4),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mkq(op: Opcode, ops: [u8; 8]) -> Quad {
        Quad {
            operation: op as u8,
            operands: ops,
        }
    }

    #[test]
    fn opcode_round_trip() {
        for b in 0u8..=47 {
            let op = Opcode::from_u8(b).unwrap();
            assert_eq!(op as u8, b);
        }
        assert!(Opcode::from_u8(48).is_none());
        assert!(Opcode::from_u8(255).is_none());
    }

    #[test]
    fn zero_arity_opcodes_decode() {
        assert_eq!(
            decode(mkq(Opcode::Empty, [0; 8])).unwrap(),
            Instruction::Empty
        );
        assert_eq!(decode(mkq(Opcode::Nop, [0; 8])).unwrap(), Instruction::Nop);
        assert_eq!(
            decode(mkq(Opcode::Return, [0; 8])).unwrap(),
            Instruction::Return
        );
        assert_eq!(
            decode(mkq(Opcode::EndFunction, [0; 8])).unwrap(),
            Instruction::EndFunction
        );
    }

    #[test]
    fn one_symbol_opcodes() {
        // sym = 0x1234 at offset 0
        let ops = [0x34, 0x12, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            decode(mkq(Opcode::Param, ops)).unwrap(),
            Instruction::Param { sym: 0x1234 }
        );
        assert_eq!(
            decode(mkq(Opcode::NativeParam, ops)).unwrap(),
            Instruction::NativeParam { sym: 0x1234 }
        );
    }

    #[test]
    fn one_address_opcodes() {
        // addr = 0xDEADBEEF at offset 0
        let ops = [0xEF, 0xBE, 0xAD, 0xDE, 0, 0, 0, 0];
        assert_eq!(
            decode(mkq(Opcode::Goto, ops)).unwrap(),
            Instruction::Goto { addr: 0xDEADBEEF }
        );
        assert_eq!(
            decode(mkq(Opcode::Call, ops)).unwrap(),
            Instruction::Call { addr: 0xDEADBEEF }
        );
        assert_eq!(
            decode(mkq(Opcode::NativeCall, ops)).unwrap(),
            Instruction::NativeCall { index: 0xDEADBEEF }
        );
    }

    #[test]
    fn two_symbol_opcodes() {
        // dst=0x0102, src=0x0304
        let ops = [0x02, 0x01, 0x04, 0x03, 0, 0, 0, 0];
        assert_eq!(
            decode(mkq(Opcode::Aff0Integer, ops)).unwrap(),
            Instruction::Aff0Integer {
                dst: 0x0102,
                src: 0x0304
            }
        );
        assert_eq!(
            decode(mkq(Opcode::BeginFunction, ops)).unwrap(),
            Instruction::BeginFunction {
                volatile_count: 0x0102,
                temp_count: 0x0304
            }
        );
    }

    #[test]
    fn sym_then_int() {
        // dst=0x0102 at offset 0, constant=-1 at offset 4
        let mut ops = [0u8; 8];
        ops[0..2].copy_from_slice(&0x0102u16.to_le_bytes());
        ops[4..8].copy_from_slice(&(-1i32).to_le_bytes());
        assert_eq!(
            decode(mkq(Opcode::Aff0IConstant, ops)).unwrap(),
            Instruction::Aff0IConstant {
                dst: 0x0102,
                constant: -1
            }
        );
    }

    #[test]
    fn sym_then_float() {
        let mut ops = [0u8; 8];
        ops[0..2].copy_from_slice(&42u16.to_le_bytes());
        ops[4..8].copy_from_slice(&3.5f32.to_le_bytes());
        assert_eq!(
            decode(mkq(Opcode::Aff0FConstant, ops)).unwrap(),
            Instruction::Aff0FConstant {
                dst: 42,
                constant: 3.5
            }
        );
    }

    #[test]
    fn sym_then_address_branches() {
        let mut ops = [0u8; 8];
        ops[0..2].copy_from_slice(&7u16.to_le_bytes());
        ops[4..8].copy_from_slice(&100u32.to_le_bytes());
        assert_eq!(
            decode(mkq(Opcode::IfZeroGoto, ops)).unwrap(),
            Instruction::IfZeroGoto { sym: 7, addr: 100 }
        );
        assert_eq!(
            decode(mkq(Opcode::IfNotZeroGoto, ops)).unwrap(),
            Instruction::IfNotZeroGoto { sym: 7, addr: 100 }
        );
    }

    #[test]
    fn ternary_binary_ops() {
        // dst=1, a=2, b=3
        let mut ops = [0u8; 8];
        ops[0..2].copy_from_slice(&1u16.to_le_bytes());
        ops[2..4].copy_from_slice(&2u16.to_le_bytes());
        ops[4..6].copy_from_slice(&3u16.to_le_bytes());
        assert_eq!(
            decode(mkq(Opcode::Aff2IAdd, ops)).unwrap(),
            Instruction::Binary {
                op: BinaryOp::IAdd,
                dst: 1,
                a: 2,
                b: 3
            }
        );
        assert_eq!(
            decode(mkq(Opcode::Aff2FEq, ops)).unwrap(),
            Instruction::Binary {
                op: BinaryOp::FEq,
                dst: 1,
                a: 2,
                b: 3
            }
        );
    }

    #[test]
    fn rejects_unknown_opcode() {
        let bad = Quad {
            operation: 99,
            operands: [0; 8],
        };
        assert_eq!(decode(bad).unwrap_err(), DecodeError::UnknownOpcode(99));
    }
}
