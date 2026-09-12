//! Byte-exact opcode fixture encoders shared by script dispatch suites.
use crate::vm::{Opcode, Quad};

#[test]
fn fixture_encoders_preserve_operand_layouts() {
    assert_eq!(
        q_begin_function(0x1234, 0x5678).operands,
        [0x34, 0x12, 0x78, 0x56, 0, 0, 0, 0]
    );
    assert_eq!(
        q_aff1_get_param(0xc004, -4).operands,
        [4, 0xc0, 0, 0, 0xfc, 0xff, 0xff, 0xff]
    );
    assert_eq!(
        q_aff0_iconstant(0xc008, 42).operands,
        [8, 0xc0, 0, 0, 42, 0, 0, 0]
    );
    assert_eq!(
        q_native_call(0x12345678).operands,
        [0x78, 0x56, 0x34, 0x12, 0, 0, 0, 0]
    );
    assert_eq!(q_return_val(0xc000).operation, Opcode::ReturnVal as u8);
    assert_eq!(q_return().operands, [0; 8]);
}
pub(crate) fn q_begin_function(volatile: u16, temp: u16) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&volatile.to_le_bytes());
    ops[2..4].copy_from_slice(&temp.to_le_bytes());
    Quad {
        operation: Opcode::BeginFunction as u8,
        operands: ops,
    }
}

pub(crate) fn q_end_function() -> Quad {
    Quad {
        operation: Opcode::EndFunction as u8,
        operands: [0u8; 8],
    }
}

pub(crate) fn q_return() -> Quad {
    Quad {
        operation: Opcode::Return as u8,
        operands: [0u8; 8],
    }
}

pub(crate) fn q_return_val(sym: u16) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&sym.to_le_bytes());
    Quad {
        operation: Opcode::ReturnVal as u8,
        operands: ops,
    }
}

pub(crate) fn q_aff1_get_param(dst: u16, param_offset: i32) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&dst.to_le_bytes());
    ops[4..8].copy_from_slice(&param_offset.to_le_bytes());
    Quad {
        operation: Opcode::Aff1GetParam as u8,
        operands: ops,
    }
}

pub(crate) fn q_aff0_iconstant(dst: u16, constant: i32) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&dst.to_le_bytes());
    ops[4..8].copy_from_slice(&constant.to_le_bytes());
    Quad {
        operation: Opcode::Aff0IConstant as u8,
        operands: ops,
    }
}

pub(crate) fn q_native_param(sym: u16) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&sym.to_le_bytes());
    Quad {
        operation: Opcode::NativeParam as u8,
        operands: ops,
    }
}

pub(crate) fn q_native_call(index: u32) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..4].copy_from_slice(&index.to_le_bytes());
    Quad {
        operation: Opcode::NativeCall as u8,
        operands: ops,
    }
}

pub(crate) fn q_aff1_native_get_return(dst: u16) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&dst.to_le_bytes());
    Quad {
        operation: Opcode::Aff1NativeGetReturn as u8,
        operands: ops,
    }
}

pub(crate) fn q_iadd(dst: u16, a: u16, b: u16) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&dst.to_le_bytes());
    ops[2..4].copy_from_slice(&a.to_le_bytes());
    ops[4..6].copy_from_slice(&b.to_le_bytes());
    Quad {
        operation: Opcode::Aff2IAdd as u8,
        operands: ops,
    }
}

pub(crate) fn q_ieq(dst: u16, a: u16, b: u16) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&dst.to_le_bytes());
    ops[2..4].copy_from_slice(&a.to_le_bytes());
    ops[4..6].copy_from_slice(&b.to_le_bytes());
    Quad {
        operation: Opcode::Aff2IEq as u8,
        operands: ops,
    }
}

pub(crate) fn q_if_not_zero_goto(sym: u16, addr: u32) -> Quad {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&sym.to_le_bytes());
    ops[4..8].copy_from_slice(&addr.to_le_bytes());
    Quad {
        operation: Opcode::IfNotZeroGoto as u8,
        operands: ops,
    }
}
