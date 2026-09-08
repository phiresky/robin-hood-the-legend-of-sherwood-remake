//! Human-readable disassembler for `.scb` scripts.
//!
//! Walks a parsed `ScbFile`, decodes each quad, and prints each
//! instruction with its address, opcode, and operands. Useful for eye-
//! balling scripts during VM bring-up and for debugging trace divergences.

use crate::scb::{ClassEntry, ScbFile};
use robin_engine::natives::native_name;
use robin_engine::vm::{BinaryOp, Instruction, Symbol, decode};
use std::fmt::Write;

/// Format a VM symbol as a human-readable region + offset.
/// E.g. 0xC004 → "tmp4", 0x4008 → "heap8".
pub fn format_sym(sym: Symbol) -> String {
    let offset = sym & 0x3FFF;
    match sym & 0xC000 {
        0x0000 => format!("static{offset}"),
        0x4000 => format!("heap{offset}"),
        0x8000 => format!("vol{offset}"),
        0xC000 => format!("tmp{offset}"),
        _ => unreachable!(),
    }
}

/// Pretty-print an entire `.scb` file — header, per class a listing of
/// members, functions, and the decoded bytecode.
pub fn dump(scb: &ScbFile) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "scb version {}  classes={}",
        scb.version,
        scb.classes.len()
    );
    for (i, c) in scb.classes.iter().enumerate() {
        let _ = writeln!(
            out,
            "\n=== class[{i}] {:?} ({}) ===",
            c.class_name, c.source_file
        );
        dump_class(&mut out, c);
    }
    out
}

fn dump_class(out: &mut String, c: &ClassEntry) {
    let _ = writeln!(
        out,
        "  members: {}  functions: {}  quads: {}  heap_size: {}",
        c.member_variables.len(),
        c.functions.len(),
        c.quads.len(),
        c.size_of_member_variables
    );

    if !c.member_variables.is_empty() {
        let _ = writeln!(out, "  --- members ---");
        for (i, m) in c.member_variables.iter().enumerate() {
            let ty_name = if m.ty.native_type_name.is_empty() {
                format!("{:?}", m.ty.tag)
            } else {
                format!("{:?}<{}>", m.ty.tag, m.ty.native_type_name)
            };
            let _ = writeln!(
                out,
                "    [{i:3}] {:<24} {ty_name:<24} @heap+{}",
                m.name, m.address
            );
        }
    }

    if !c.functions.is_empty() {
        let _ = writeln!(out, "  --- functions ---");
        for (i, f) in c.functions.iter().enumerate() {
            let _ = writeln!(
                out,
                "    [{i:3}] {:<28} addr={:<5} params={} vol={} tmp={} ret={} pbytes={}",
                f.name,
                f.address,
                f.num_parameters,
                f.size_of_volatile,
                f.size_of_temporary,
                f.size_of_return_value,
                f.size_of_parameters
            );
        }
    }

    if !c.quads.is_empty() {
        let _ = writeln!(out, "  --- bytecode ---");
        let decoded: Vec<_> = c.quads.iter().map(|q| decode(*q)).collect();
        let mut i = 0;
        while i < decoded.len() {
            // Group NativeParam* NativeCall NativeGetReturn? into one line
            if matches!(&decoded[i], Ok(Instruction::NativeParam { .. })) {
                let start = i;
                let mut params = Vec::new();
                while i < decoded.len() {
                    if let Ok(Instruction::NativeParam { sym }) = &decoded[i] {
                        params.push(*sym);
                        i += 1;
                    } else {
                        break;
                    }
                }
                if let Some(Ok(Instruction::NativeCall { index })) = decoded.get(i) {
                    let index = *index;
                    i += 1;
                    let ret_sym = if let Some(Ok(Instruction::Aff1NativeGetReturn { sym })) =
                        decoded.get(i)
                    {
                        i += 1;
                        Some(*sym)
                    } else {
                        None
                    };
                    let args = params
                        .iter()
                        .map(|s| format_sym(*s))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let name = native_name(index);
                    let call = if let Some(ret) = ret_sym {
                        format!("{} = {name}({args})", format_sym(ret))
                    } else {
                        format!("{name}({args})")
                    };
                    let _ = writeln!(out, "    {start:5}  {call:<50} // native #{index}");
                    continue;
                }
                // NativeParam not followed by NativeCall — emit individually
                for (j, sym) in params.iter().enumerate() {
                    let ins = Instruction::NativeParam { sym: *sym };
                    let _ = writeln!(out, "    {:5}  {}", start + j, format_ins(&ins));
                }
                continue;
            }
            // Bare NativeCall (no params)
            if let Ok(Instruction::NativeCall { index }) = &decoded[i] {
                let index = *index;
                let start = i;
                i += 1;
                let ret_sym =
                    if let Some(Ok(Instruction::Aff1NativeGetReturn { sym })) = decoded.get(i) {
                        i += 1;
                        Some(*sym)
                    } else {
                        None
                    };
                let name = native_name(index);
                let call = if let Some(ret) = ret_sym {
                    format!("{} = {name}()", format_sym(ret))
                } else {
                    format!("{name}()")
                };
                let _ = writeln!(out, "    {start:5}  {call:<50} // native #{index}");
                continue;
            }
            match &decoded[i] {
                Ok(ins) => {
                    let _ = writeln!(out, "    {i:5}  {}", format_ins(ins));
                }
                Err(e) => {
                    let _ = writeln!(
                        out,
                        "    {i:5}  <bad op=0x{:02x}: {e:?}>",
                        c.quads[i].operation
                    );
                }
            }
            i += 1;
        }
    }
}

/// Format a single instruction as a short mnemonic + operands. Meant
/// for diagnostics, not for round-tripping.
pub fn format_ins(ins: &Instruction) -> String {
    use Instruction::*;
    let s = format_sym;
    match ins {
        Empty => "EMPTY".into(),
        Nop => "NOP".into(),
        EndFunction => "END_FUNCTION".into(),
        Return => "RETURN".into(),
        Param { sym } => format!("PARAM        {}", s(*sym)),
        ReturnVal { sym } => format!("RETURNVAL    {}", s(*sym)),
        NativeParam { sym } => format!("NATIVEPARAM  {}", s(*sym)),
        Aff1NativeGetReturn { sym } => format!("NATRET->     {}", s(*sym)),
        Aff1GetReturn { sym } => format!("GETRET->     {}", s(*sym)),
        BeginFunction {
            volatile_count,
            temp_count,
        } => {
            format!("BEGIN_FN     vol={volatile_count} tmp={temp_count}")
        }
        Aff1CastToInt { dst, src } => format!("CAST_INT     {} = (int){}", s(*dst), s(*src)),
        Aff1CastToFloat { dst, src } => format!("CAST_FLT     {} = (float){}", s(*dst), s(*src)),
        Aff1IMinus { dst, src } => format!("INEG         {} = -{}", s(*dst), s(*src)),
        Aff1FMinus { dst, src } => format!("FNEG         {} = -{}", s(*dst), s(*src)),
        Aff0Integer { dst, src } => format!("IMOV         {} = {}", s(*dst), s(*src)),
        Aff0Float { dst, src } => format!("FMOV         {} = {}", s(*dst), s(*src)),
        Goto { addr } => format!("GOTO         @{addr}"),
        Call { addr } => format!("CALL         @{addr}"),
        NativeCall { index } => format!("NATIVE_CALL  #{index} {}", native_name(*index)),
        Aff0IConstant { dst, constant } => format!("ICONST       {} = {constant}", s(*dst)),
        Aff1GetParam { dst, param_offset } => {
            format!("GETPARAM     {} = param[{param_offset}]", s(*dst))
        }
        Aff1SetParam { dst_offset, src } => {
            format!("SETPARAM     param[{dst_offset}] = {}", s(*src))
        }
        IfZeroGoto { sym, addr } => format!("IFZ          {} goto @{addr}", s(*sym)),
        IfNotZeroGoto { sym, addr } => format!("IFNZ         {} goto @{addr}", s(*sym)),
        Aff0FConstant { dst, constant } => format!("FCONST       {} = {constant}", s(*dst)),
        Binary { op, dst, a, b } => format!(
            "{:<12} {} = {} {} {}",
            bin_mnemonic(*op),
            s(*dst),
            s(*a),
            bin_symbol(*op),
            s(*b)
        ),
    }
}

fn bin_mnemonic(op: BinaryOp) -> &'static str {
    use BinaryOp::*;
    match op {
        IAdd => "IADD",
        ISub => "ISUB",
        IMult => "IMUL",
        IDiv => "IDIV",
        IAor => "IOR",
        IAand => "IAND",
        IAxor => "IXOR",
        FAdd => "FADD",
        FSub => "FSUB",
        FMult => "FMUL",
        FDiv => "FDIV",
        IInfEq => "ILE",
        IInf => "ILT",
        ISupEq => "IGE",
        ISup => "IGT",
        INeq => "INE",
        IEq => "IEQ",
        FInfEq => "FLE",
        FInf => "FLT",
        FSupEq => "FGE",
        FSup => "FGT",
        FNeq => "FNE",
        FEq => "FEQ",
    }
}

fn bin_symbol(op: BinaryOp) -> &'static str {
    use BinaryOp::*;
    match op {
        IAdd | FAdd => "+",
        ISub | FSub => "-",
        IMult | FMult => "*",
        IDiv | FDiv => "/",
        IAor => "|",
        IAand => "&",
        IAxor => "^",
        IInfEq | FInfEq => "<=",
        IInf | FInf => "<",
        ISupEq | FSupEq => ">=",
        ISup | FSup => ">",
        INeq | FNeq => "!=",
        IEq | FEq => "==",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scb::{self, ClassEntry, Function, MemberVariable, Quad, ScType, ScbFile, TypeTag};
    use robin_engine::vm::Opcode;

    #[test]
    fn formats_every_instruction_variant() {
        use BinaryOp::*;
        use Instruction::*;
        // Sanity-check that format_ins doesn't panic on any variant. Not
        // an exhaustive content check — just "I wrote a match arm for
        // each".
        let samples = [
            Empty,
            Nop,
            EndFunction,
            Return,
            Param { sym: 1 },
            ReturnVal { sym: 2 },
            NativeParam { sym: 3 },
            Aff1NativeGetReturn { sym: 4 },
            Aff1GetReturn { sym: 5 },
            BeginFunction {
                volatile_count: 6,
                temp_count: 7,
            },
            Aff1CastToInt { dst: 1, src: 2 },
            Aff1CastToFloat { dst: 1, src: 2 },
            Aff1IMinus { dst: 1, src: 2 },
            Aff1FMinus { dst: 1, src: 2 },
            Aff0Integer { dst: 1, src: 2 },
            Aff0Float { dst: 1, src: 2 },
            Goto { addr: 100 },
            Call { addr: 200 },
            NativeCall { index: 3 },
            Aff0IConstant {
                dst: 1,
                constant: -5,
            },
            Aff1GetParam {
                dst: 1,
                param_offset: 0,
            },
            Aff1SetParam {
                dst_offset: 0,
                src: 1,
            },
            IfZeroGoto { sym: 1, addr: 10 },
            IfNotZeroGoto { sym: 1, addr: 10 },
            Aff0FConstant {
                dst: 1,
                constant: 3.5,
            },
            Binary {
                op: IAdd,
                dst: 1,
                a: 2,
                b: 3,
            },
            Binary {
                op: FEq,
                dst: 1,
                a: 2,
                b: 3,
            },
        ];
        for ins in samples.iter() {
            let s = format_ins(ins);
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn dump_has_header_and_class_section() {
        let scb = ScbFile {
            version: 1.5,
            classes: vec![ClassEntry {
                source_file: "script.scs".into(),
                class_name: "Demo".into(),
                size_of_member_variables: 4,
                member_variables: vec![MemberVariable {
                    ty: ScType {
                        tag: TypeTag::Int,
                        native_type_name: String::new(),
                    },
                    name: "x".into(),
                    address: 0,
                }],
                functions: vec![Function {
                    name: "Main".into(),
                    address: 0,
                    num_parameters: 0,
                    size_of_return_value: 0,
                    size_of_parameters: 0,
                    size_of_volatile: 0,
                    size_of_temporary: 0,
                }],
                quads: vec![
                    Quad {
                        operation: Opcode::Nop as u8,
                        operands: [0; 8],
                    },
                    Quad {
                        operation: Opcode::Return as u8,
                        operands: [0; 8],
                    },
                ],
            }],
        };
        let text = dump(&scb);
        assert!(text.contains("scb version 1.5"));
        assert!(text.contains("class[0] \"Demo\""));
        assert!(text.contains("x"));
        assert!(text.contains("Main"));
        assert!(text.contains("NOP"));
        assert!(text.contains("RETURN"));
    }

    /// Smoke test: the demo script dumps without panicking and produces
    /// meaningful output. Skipped if assets aren't present.
    #[test]
    fn dumps_shipped_demo_script() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let path = std::path::PathBuf::from(manifest_dir)
            .join("../../datadirs/demo/Data/Levels/Dem_Lei_MP.scb");
        let Ok(path) = path.canonicalize() else {
            return;
        };
        let scb = scb::parse_file(&path).unwrap();
        let text = dump(&scb);
        assert!(text.contains("StartUp"));
        assert!(text.contains("--- bytecode ---"));
        // A 65KB script will produce a big dump. Sanity-bound it.
        assert!(
            text.len() > 1000,
            "dump seems suspiciously small: {}",
            text.len()
        );
    }
}
