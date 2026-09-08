//! Decompiler: transforms SCB bytecode into JS-like pseudo-code.
//!
//! Provides a higher-level view than the disassembler by:
//! - Folding constants and expressions (eliminating temporaries)
//! - Inlining native call arguments
//! - Recovering if/else and while control flow
//! - Using member variable names from class metadata

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::{self, Write};

use crate::actor_names::{ActorNames, ScriptKind};
use crate::scb::{ClassEntry, ScbFile};
use robin_engine::natives::{native_name, native_signature_by_index};
use robin_engine::vm::{BinaryOp, Instruction, Symbol, decode};

// ── Known parameter names for script functions ─────────────────
//
// Key: function name → list of parameter names.

/// Known parameter names for script functions, keyed by (class_name, fn_name).
///
/// Different class types have different signatures for the same function name.
/// The StartUp class (engine script) includes `this` as an explicit parameter,
/// while actor/target/zone/scroll classes do not.
///
/// We match on class name prefix: "StartUp" for engine scripts, and fall
/// through to function-name-only matches for actor/target/zone/scroll scripts.
fn known_function_params(
    class_name: &str,
    fn_name: &str,
    param_count: usize,
) -> Option<Vec<&'static str>> {
    // Engine script (StartUp class).
    // The VM pushes `this` as a hidden param for functions that also have
    // explicit params (Initialize, Hourglass, etc.) but NOT for functions
    // with zero explicit params (PostInitialize, Briefing).
    if class_name == "StartUp" {
        return Some(match (fn_name, param_count) {
            ("Initialize", 2) => vec!["this", "seed"],
            ("Initialize", 1) => vec!["seed"],
            ("Hourglass", 2) => vec!["this", "time_seconds"],
            ("Hourglass", 1) => vec!["time_seconds"],
            ("CheckVictoryCondition", 2) => vec!["this", "time_seconds"],
            ("CheckVictoryCondition", 1) => vec!["time_seconds"],
            ("Finalize", 2) => vec!["this", "abandoned"],
            ("Finalize", 1) => vec!["abandoned"],
            ("PostInitialize", _) => vec![],
            ("Briefing", _) => vec![],
            ("ProcessMessage", 4) => vec!["this", "message_code", "arg1", "arg2"],
            ("ProcessMessage", 3) => vec!["message_code", "arg1", "arg2"],
            _ => return None,
        });
    }

    // Actor/target/zone/scroll scripts.
    // `this` is included as a parameter in some cases depending on the
    // bytecode. We define the "explicit" param names and match with or
    // without a leading `this` based on actual param_count.
    let explicit: &[&str] = match fn_name {
        "ActionChange" => &["action", "old_action"],
        "FilterAIEvent" => &["source_actor", "event_code"],
        "HandleEvent" => &["source_actor", "event_code"],
        "ProcessMessage" => &["message_code", "arg1", "arg2"],

        "ActivatedByApple" | "ActivatedByArrow" | "ActivatedByHand" | "ActivatedByHeal"
        | "ActivatedByLever" | "ActivatedByMoney" | "ActivatedBySearch" | "ActivatedByStone"
        | "ActivatedBySword" | "ActivatedByNet" => &["actor"],

        "EnterZone" | "ExitZone" => &["actor"],
        "ReachedWaypoint" => &["actor"],
        "IsTaken" => &["actor"],
        "Hourglass" => &["time_seconds"],

        _ => return None,
    };

    if explicit.len() == param_count {
        Some(explicit.to_vec())
    } else if explicit.len() + 1 == param_count {
        let mut v = vec!["this"];
        v.extend_from_slice(explicit);
        Some(v)
    } else {
        // Mismatch we can't explain — return what we have, warning will fire
        Some(explicit.to_vec())
    }
}

// ── Expression tree ──────────────────────────────────────────

#[derive(Clone, Debug)]
enum Expr {
    Int(i32),
    Float(f32),
    Var(String),
    Call(String, Vec<Expr>),
    BinOp(&'static str, Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
    Cast(&'static str, Box<Expr>),
    /// Named argument: `name: expr` (for known native/script param names)
    NamedArg(String, Box<Expr>),
    /// Expression with a trailing `/* comment */`. Used for resolved
    /// resource IDs (popup text, short briefing, …) where we want to
    /// show the string content alongside the literal index.
    WithTrailingComment(Box<Expr>, String),
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Int(v) => write!(f, "{v}"),
            Expr::Float(v) => {
                if v.fract() == 0.0 {
                    write!(f, "{v:.1}")
                } else {
                    write!(f, "{v}")
                }
            }
            Expr::Var(n) => f.write_str(n),
            Expr::Call(name, args) => {
                write!(f, "{name}(")?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{a}")?;
                }
                write!(f, ")")
            }
            Expr::BinOp(op, l, r) => write!(f, "({l} {op} {r})"),
            Expr::Neg(e) => write!(f, "(-{e})"),
            Expr::Cast(ty, e) => write!(f, "{ty}({e})"),
            Expr::NamedArg(name, e) => write!(f, "/*{name}*/ {e}"),
            Expr::WithTrailingComment(e, comment) => write!(f, "{e} /* {comment} */"),
        }
    }
}

/// Strip outer parentheses if they are balanced.
fn strip_parens(s: &str) -> &str {
    let s = s.trim();
    if s.starts_with('(') && s.ends_with(')') {
        let inner = &s[1..s.len() - 1];
        let mut depth = 0i32;
        for ch in inner.chars() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth < 0 {
                        return s;
                    }
                }
                _ => {}
            }
        }
        if depth == 0 { inner } else { s }
    } else {
        s
    }
}

// ── Flat IR (before control-flow recovery) ───────────────────

#[derive(Clone, Debug)]
enum FlatIR {
    Expr(Expr),
    Assign(String, Expr),
    Return(Option<Expr>),
    Branch {
        cond: Expr,
        true_target: usize,
        false_target: usize,
    },
    Goto(usize),
}

// ── Structured statements ────────────────────────────────────

#[derive(Clone, Debug)]
enum Stmt {
    Expr(Expr),
    Assign(String, Expr),
    Return(Option<Expr>),
    If {
        cond: Expr,
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    Goto(usize),
    Label(usize),
}

// ── Symbol naming ────────────────────────────────────────────

fn is_tmp(sym: Symbol) -> bool {
    sym & 0xC000 == 0xC000
}

fn sym_var_name(sym: Symbol, members: &HashMap<usize, &str>, _param_names: &[String]) -> String {
    let offset = (sym & 0x3FFF) as usize;
    match sym & 0xC000 {
        0x0000 => format!("g_{offset}"),
        0x4000 => members
            .get(&offset)
            .map(|n| format!("this.{n}"))
            .unwrap_or_else(|| format!("this.heap_{offset}")),
        // Volatile locals — a separate frame region from parameters. Params
        // are only reachable via GETPARAM/SETPARAM (see Aff1GetParam), never
        // via a 0x8000 symbol, so we must not index param_names here.
        0x8000 => format!("v{}", offset / 4),
        0xC000 => format!("t{}", offset / 4),
        _ => unreachable!(),
    }
}

fn bin_op_str(op: BinaryOp) -> &'static str {
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

// ── Expression folding pass ──────────────────────────────────

fn get_expr(
    exprs: &HashMap<Symbol, Expr>,
    sym: Symbol,
    members: &HashMap<usize, &str>,
    param_names: &[String],
) -> Expr {
    exprs
        .get(&sym)
        .cloned()
        .unwrap_or_else(|| Expr::Var(sym_var_name(sym, members, param_names)))
}

/// Store an expression for `dst`. If `dst` is a real variable (not tmp),
/// emit an Assign to the flat IR, and record the *variable name* (not the
/// full expression) so later reads reference the variable, not the inlined
/// constant. This is critical for loop counters, accumulators, etc.
/// Pre-pass: find tmp symbols that are read more than once between writes.
/// Tmp slots are reused across a function, so we must reset the read counter
/// on each write to avoid false positives.
fn count_multi_use_tmps(instructions: &[Instruction], start: usize, end: usize) -> HashSet<Symbol> {
    let mut reads_since_write: HashMap<Symbol, usize> = HashMap::new();
    let mut is_multi_use: HashSet<Symbol> = HashSet::new();
    let end = end.min(instructions.len());
    use Instruction::*;

    fn count_read(reads: &mut HashMap<Symbol, usize>, multi: &mut HashSet<Symbol>, sym: Symbol) {
        if is_tmp(sym) {
            let c = reads.entry(sym).or_insert(0);
            *c += 1;
            if *c > 1 {
                multi.insert(sym);
            }
        }
    }
    fn count_write(reads: &mut HashMap<Symbol, usize>, sym: Symbol) {
        if is_tmp(sym) {
            reads.insert(sym, 0);
        }
    }

    for instruction in &instructions[start..end] {
        match instruction {
            Aff0Integer { src, dst } | Aff0Float { src, dst } => {
                count_read(&mut reads_since_write, &mut is_multi_use, *src);
                count_write(&mut reads_since_write, *dst);
            }
            Binary { a, b, dst, .. } => {
                count_read(&mut reads_since_write, &mut is_multi_use, *a);
                count_read(&mut reads_since_write, &mut is_multi_use, *b);
                count_write(&mut reads_since_write, *dst);
            }
            Aff1CastToInt { src, dst } | Aff1CastToFloat { src, dst } => {
                count_read(&mut reads_since_write, &mut is_multi_use, *src);
                count_write(&mut reads_since_write, *dst);
            }
            Aff1IMinus { src, dst } | Aff1FMinus { src, dst } => {
                count_read(&mut reads_since_write, &mut is_multi_use, *src);
                count_write(&mut reads_since_write, *dst);
            }
            Aff1SetParam { src, .. } => {
                count_read(&mut reads_since_write, &mut is_multi_use, *src);
            }
            NativeParam { sym } => {
                count_read(&mut reads_since_write, &mut is_multi_use, *sym);
            }
            Param { sym } => {
                count_read(&mut reads_since_write, &mut is_multi_use, *sym);
            }
            ReturnVal { sym } => {
                count_read(&mut reads_since_write, &mut is_multi_use, *sym);
            }
            IfNotZeroGoto { sym, .. } | IfZeroGoto { sym, .. } => {
                count_read(&mut reads_since_write, &mut is_multi_use, *sym);
            }
            // Write-only instructions
            Aff0IConstant { dst, .. } | Aff0FConstant { dst, .. } => {
                count_write(&mut reads_since_write, *dst);
            }
            Aff1GetParam { dst, .. } => {
                count_write(&mut reads_since_write, *dst);
            }
            Aff1NativeGetReturn { sym } | Aff1GetReturn { sym } => {
                count_write(&mut reads_since_write, *sym);
            }
            _ => {}
        }
    }
    is_multi_use
}

/// Does this expression tree contain a function call (side effect)?
/// Pure expressions (constants, vars, arithmetic) can safely be inlined
/// multiple times; calls cannot.
fn has_side_effect(expr: &Expr) -> bool {
    match expr {
        Expr::Int(_) | Expr::Float(_) | Expr::Var(_) => false,
        Expr::Call(_, _) => true,
        Expr::BinOp(_, l, r) => has_side_effect(l) || has_side_effect(r),
        Expr::Neg(e) | Expr::Cast(_, e) | Expr::NamedArg(_, e) => has_side_effect(e),
        Expr::WithTrailingComment(e, _) => has_side_effect(e),
    }
}

#[allow(clippy::too_many_arguments)]
fn set_expr(
    flat: &mut BTreeMap<usize, FlatIR>,
    exprs: &mut HashMap<Symbol, Expr>,
    addr: usize,
    dst: Symbol,
    expr: Expr,
    members: &HashMap<usize, &str>,
    multi_use: &HashSet<Symbol>,
    param_names: &[String],
) {
    if !is_tmp(dst) || (multi_use.contains(&dst) && has_side_effect(&expr)) {
        let name = sym_var_name(dst, members, param_names);
        flat.insert(addr, FlatIR::Assign(name.clone(), expr));
        exprs.insert(dst, Expr::Var(name));
    } else {
        exprs.insert(dst, expr);
    }
}

fn fold_expressions(
    instructions: &[Instruction],
    start: usize,
    end: usize,
    members: &HashMap<usize, &str>,
    func_map: &HashMap<usize, &str>,
    param_names: &[String],
) -> BTreeMap<usize, FlatIR> {
    let mut flat = BTreeMap::new();
    let mut exprs: HashMap<Symbol, Expr> = HashMap::new();
    let mut native_params: Vec<Expr> = Vec::new();
    let mut script_params: Vec<Expr> = Vec::new();

    let end = end.min(instructions.len());
    let multi_use = count_multi_use_tmps(instructions, start, end);
    let mut i = start;
    while i < end {
        use Instruction::*;
        match &instructions[i] {
            Empty | Nop | EndFunction => {
                i += 1;
            }
            BeginFunction { .. } => {
                i += 1;
            }

            Aff0IConstant { dst, constant } => {
                set_expr(
                    &mut flat,
                    &mut exprs,
                    i,
                    *dst,
                    Expr::Int(*constant),
                    members,
                    &multi_use,
                    param_names,
                );
                i += 1;
            }
            Aff0FConstant { dst, constant } => {
                set_expr(
                    &mut flat,
                    &mut exprs,
                    i,
                    *dst,
                    Expr::Float(*constant),
                    members,
                    &multi_use,
                    param_names,
                );
                i += 1;
            }
            Aff0Integer { dst, src } | Aff0Float { dst, src } => {
                let e = get_expr(&exprs, *src, members, param_names);
                set_expr(
                    &mut flat,
                    &mut exprs,
                    i,
                    *dst,
                    e,
                    members,
                    &multi_use,
                    param_names,
                );
                i += 1;
            }
            Binary { op, dst, a, b } => {
                let le = get_expr(&exprs, *a, members, param_names);
                let re = get_expr(&exprs, *b, members, param_names);
                let e = Expr::BinOp(bin_op_str(*op), Box::new(le), Box::new(re));
                set_expr(
                    &mut flat,
                    &mut exprs,
                    i,
                    *dst,
                    e,
                    members,
                    &multi_use,
                    param_names,
                );
                i += 1;
            }
            Aff1CastToInt { dst, src } => {
                let e = Expr::Cast(
                    "int",
                    Box::new(get_expr(&exprs, *src, members, param_names)),
                );
                set_expr(
                    &mut flat,
                    &mut exprs,
                    i,
                    *dst,
                    e,
                    members,
                    &multi_use,
                    param_names,
                );
                i += 1;
            }
            Aff1CastToFloat { dst, src } => {
                let e = Expr::Cast(
                    "float",
                    Box::new(get_expr(&exprs, *src, members, param_names)),
                );
                set_expr(
                    &mut flat,
                    &mut exprs,
                    i,
                    *dst,
                    e,
                    members,
                    &multi_use,
                    param_names,
                );
                i += 1;
            }
            Aff1IMinus { dst, src } | Aff1FMinus { dst, src } => {
                let e = Expr::Neg(Box::new(get_expr(&exprs, *src, members, param_names)));
                set_expr(
                    &mut flat,
                    &mut exprs,
                    i,
                    *dst,
                    e,
                    members,
                    &multi_use,
                    param_names,
                );
                i += 1;
            }
            Aff1GetParam { dst, param_offset } => {
                let slot = param_offset / 4;
                let name = if param_offset % 4 == 0 {
                    param_names
                        .get(slot as usize)
                        .cloned()
                        .unwrap_or_else(|| format!("p{slot}"))
                } else {
                    format!("param[{param_offset}]")
                };
                set_expr(
                    &mut flat,
                    &mut exprs,
                    i,
                    *dst,
                    Expr::Var(name),
                    members,
                    &multi_use,
                    param_names,
                );
                i += 1;
            }
            Aff1SetParam { dst_offset, src } => {
                let e = get_expr(&exprs, *src, members, param_names);
                flat.insert(i, FlatIR::Assign(format!("out_param[{dst_offset}]"), e));
                i += 1;
            }

            NativeParam { sym } => {
                native_params.push(get_expr(&exprs, *sym, members, param_names));
                i += 1;
            }
            NativeCall { index } => {
                let name = native_name(*index).to_string();
                let mut args = std::mem::take(&mut native_params);

                // Annotate arguments with known parameter names
                if let Some(sig) = native_signature_by_index(*index) {
                    if sig.params.len() != args.len() {
                        tracing::warn!(
                            "Decompiler: native {}({}) has {} args but {} known names",
                            name,
                            index,
                            args.len(),
                            sig.params.len(),
                        );
                    }
                    for (i, arg) in args.iter_mut().enumerate() {
                        if let Some(param) = sig.params.get(i) {
                            *arg = Expr::NamedArg(param.name.to_string(), Box::new(arg.clone()));
                        }
                    }
                }

                let call = Expr::Call(name, args);

                if i + 1 < end
                    && let Aff1NativeGetReturn { sym } = &instructions[i + 1]
                {
                    set_expr(
                        &mut flat,
                        &mut exprs,
                        i,
                        *sym,
                        call,
                        members,
                        &multi_use,
                        param_names,
                    );
                    i += 2;
                    continue;
                }
                flat.insert(i, FlatIR::Expr(call));
                i += 1;
            }
            Aff1NativeGetReturn { sym } => {
                // Stray (should have been consumed by NativeCall handler).
                let e = Expr::Var("__native_ret".into());
                set_expr(
                    &mut flat,
                    &mut exprs,
                    i,
                    *sym,
                    e,
                    members,
                    &multi_use,
                    param_names,
                );
                i += 1;
            }

            Param { sym } => {
                script_params.push(get_expr(&exprs, *sym, members, param_names));
                i += 1;
            }
            Call { addr } => {
                let target = *addr as usize;
                let name = func_map
                    .get(&target)
                    .map(|s| format!("this.{s}"))
                    .unwrap_or_else(|| format!("func_{target}"));
                let args = std::mem::take(&mut script_params);
                let call = Expr::Call(name, args);

                if i + 1 < end
                    && let Aff1GetReturn { sym } = &instructions[i + 1]
                {
                    set_expr(
                        &mut flat,
                        &mut exprs,
                        i,
                        *sym,
                        call,
                        members,
                        &multi_use,
                        param_names,
                    );
                    i += 2;
                    continue;
                }
                flat.insert(i, FlatIR::Expr(call));
                i += 1;
            }
            Aff1GetReturn { sym } => {
                let e = Expr::Var("__call_ret".into());
                set_expr(
                    &mut flat,
                    &mut exprs,
                    i,
                    *sym,
                    e,
                    members,
                    &multi_use,
                    param_names,
                );
                i += 1;
            }

            ReturnVal { sym } => {
                let e = get_expr(&exprs, *sym, members, param_names);
                flat.insert(i, FlatIR::Return(Some(e)));
                i += 1;
                // Skip compiler epilogue: NOP, RETURN, END_FUNCTION
                while i < end && matches!(&instructions[i], Nop | Return | EndFunction) {
                    i += 1;
                }
            }
            Return => {
                // Only emit if it's NOT right after a ReturnVal (those are
                // already emitted). Check by seeing if the previous flat IR
                // entry was a Return.
                let dominated_by_ret = flat
                    .values()
                    .next_back()
                    .map(|ir| matches!(ir, FlatIR::Return(Some(_))))
                    .unwrap_or(false);
                if !dominated_by_ret {
                    // Don't emit the trailing return at end of function either —
                    // check if this is followed only by NOPs/END_FUNCTION.
                    let mut j = i + 1;
                    while j < end && matches!(&instructions[j], Nop | Return | EndFunction) {
                        j += 1;
                    }
                    if j < end {
                        // There's more code after — this is a mid-function return.
                        flat.insert(i, FlatIR::Return(None));
                    }
                }
                i += 1;
            }

            IfNotZeroGoto { sym, addr } => {
                let cond = get_expr(&exprs, *sym, members, param_names);
                let true_target = *addr as usize;
                let branch_addr = i;

                if i + 1 < end
                    && let Goto {
                        addr: false_addr, ..
                    } = &instructions[i + 1]
                {
                    let false_target = *false_addr as usize;
                    flat.insert(
                        branch_addr,
                        FlatIR::Branch {
                            cond,
                            true_target,
                            false_target,
                        },
                    );
                    i += 2;
                    continue;
                }
                flat.insert(
                    branch_addr,
                    FlatIR::Branch {
                        cond,
                        true_target,
                        false_target: i + 1,
                    },
                );
                i += 1;
            }
            IfZeroGoto { sym, addr } => {
                let cond = get_expr(&exprs, *sym, members, param_names);
                let zero_target = *addr as usize;
                let branch_addr = i;

                if i + 1 < end
                    && let Goto {
                        addr: nonzero_addr, ..
                    } = &instructions[i + 1]
                {
                    let nonzero_target = *nonzero_addr as usize;
                    // Normalise: true (nonzero) → nonzero_target
                    flat.insert(
                        branch_addr,
                        FlatIR::Branch {
                            cond,
                            true_target: nonzero_target,
                            false_target: zero_target,
                        },
                    );
                    i += 2;
                    continue;
                }
                flat.insert(
                    branch_addr,
                    FlatIR::Branch {
                        cond,
                        true_target: i + 1,
                        false_target: zero_target,
                    },
                );
                i += 1;
            }

            Goto { addr } => {
                flat.insert(i, FlatIR::Goto(*addr as usize));
                i += 1;
            }
        }
    }
    flat
}

// ── Control-flow structuring ─────────────────────────────────

fn negate_cond(cond: &Expr) -> Expr {
    match cond {
        Expr::BinOp("==", l, r) => Expr::BinOp("!=", l.clone(), r.clone()),
        Expr::BinOp("!=", l, r) => Expr::BinOp("==", l.clone(), r.clone()),
        Expr::BinOp("<", l, r) => Expr::BinOp(">=", l.clone(), r.clone()),
        Expr::BinOp(">=", l, r) => Expr::BinOp("<", l.clone(), r.clone()),
        Expr::BinOp(">", l, r) => Expr::BinOp("<=", l.clone(), r.clone()),
        Expr::BinOp("<=", l, r) => Expr::BinOp(">", l.clone(), r.clone()),
        other => Expr::BinOp("==", Box::new(other.clone()), Box::new(Expr::Int(0))),
    }
}

/// Find the next flat-IR address >= `target` within `addrs`.
fn advance_to(addrs: &[usize], target: usize) -> usize {
    addrs
        .iter()
        .position(|a| *a >= target)
        .unwrap_or(addrs.len())
}

fn structure_range(flat: &BTreeMap<usize, FlatIR>, start: usize, end: usize) -> Vec<Stmt> {
    let targets = collect_goto_targets(flat);
    structure_range_d(flat, start, end, 0, &targets)
}

fn collect_goto_targets(flat: &BTreeMap<usize, FlatIR>) -> HashSet<usize> {
    let mut targets = HashSet::new();
    for ir in flat.values() {
        match ir {
            FlatIR::Branch {
                true_target,
                false_target,
                ..
            } => {
                targets.insert(*true_target);
                targets.insert(*false_target);
            }
            FlatIR::Goto(t) => {
                targets.insert(*t);
            }
            _ => {}
        }
    }
    targets
}

fn structure_range_d(
    flat: &BTreeMap<usize, FlatIR>,
    start: usize,
    end: usize,
    depth: usize,
    targets: &HashSet<usize>,
) -> Vec<Stmt> {
    // Guard: pathological bytecode (e.g. the sherwood hub) can recursively
    // call us with `start > end`, which `BTreeMap::range` panics on. Clamp
    // to an empty body instead — better than aborting the whole batch.
    if start >= end {
        return Vec::new();
    }
    if depth > 80 {
        // Bail out to avoid stack overflow on deeply-nested chains.
        let mut result = Vec::new();
        for (&addr, ir) in flat.range(start..end) {
            if targets.contains(&addr) {
                result.push(Stmt::Label(addr));
            }
            match ir {
                FlatIR::Expr(e) => result.push(Stmt::Expr(e.clone())),
                FlatIR::Assign(n, e) => result.push(Stmt::Assign(n.clone(), e.clone())),
                FlatIR::Return(e) => result.push(Stmt::Return(e.clone())),
                FlatIR::Branch { true_target, .. } => result.push(Stmt::Goto(*true_target)),
                FlatIR::Goto(t) => result.push(Stmt::Goto(*t)),
            }
        }
        return result;
    }
    // Merge flat entries and goto-target addresses: some instructions
    // (`GETPARAM`, `ICONST`, `NOP`, …) fold into expressions and never
    // produce a flat entry, but they can still be the target of a goto
    // (typical in switch-dispatch tables). Iterating both lets us emit
    // a `Label` statement at those addresses even though there's no
    // flat IR to execute.
    let mut addrs_set: BTreeSet<usize> = flat.range(start..end).map(|(a, _)| *a).collect();
    for &t in targets {
        if t >= start && t < end {
            addrs_set.insert(t);
        }
    }
    let addrs: Vec<usize> = addrs_set.into_iter().collect();
    let mut result = Vec::new();
    let mut idx = 0;

    while idx < addrs.len() {
        let addr = addrs[idx];
        if addr >= end {
            break;
        }

        // Emit label if this address is a goto/branch target
        if targets.contains(&addr) {
            result.push(Stmt::Label(addr));
        }

        // Target address without a corresponding flat entry (folded
        // instruction like GETPARAM/ICONST). Label already emitted above.
        let Some(ir) = flat.get(&addr) else {
            idx += 1;
            continue;
        };

        match ir {
            FlatIR::Branch {
                cond,
                true_target,
                false_target,
            } => {
                let true_t = *true_target;
                let false_t = *false_target;

                // Guard: targets outside our processable range.
                if true_t >= end && false_t >= end {
                    result.push(Stmt::Goto(true_t));
                    idx += 1;
                    continue;
                }
                // Backward true-target (switch dispatch, do-while, etc.)
                // — emit as conditional goto, don't try to structure.
                if true_t <= addr {
                    result.push(Stmt::If {
                        cond: cond.clone(),
                        then_body: vec![Stmt::Goto(true_t)],
                        else_body: vec![],
                    });
                    if false_t > addr {
                        idx = advance_to(&addrs, false_t);
                    } else {
                        idx += 1;
                    }
                    continue;
                }
                // Backward false-target — same treatment with negated cond.
                if false_t <= addr {
                    result.push(Stmt::If {
                        cond: negate_cond(cond),
                        then_body: vec![Stmt::Goto(false_t)],
                        else_body: vec![],
                    });
                    idx = advance_to(&addrs, true_t);
                    continue;
                }

                if true_t < false_t {
                    // ── Standard: true_target is the closer (then body) ──
                    let then_addrs: Vec<usize> =
                        flat.range(true_t..false_t).map(|(a, _)| *a).collect();

                    // While-loop: then-body ends with backward goto?
                    let back_goto = then_addrs.last().and_then(|&a| {
                        if let FlatIR::Goto(t) = flat[&a]
                            && t <= addr
                        {
                            return Some(t);
                        }
                        None
                    });

                    if back_goto.is_some() {
                        let body_last = *then_addrs.last().unwrap();
                        let body = structure_range_d(flat, true_t, body_last, depth + 1, targets);
                        result.push(Stmt::While {
                            cond: cond.clone(),
                            body,
                        });
                        idx = advance_to(&addrs, false_t);
                        continue;
                    }

                    // If-then-else: then-body ends with forward goto past false_t?
                    let else_goto = then_addrs.last().and_then(|&a| {
                        if let FlatIR::Goto(t) = flat[&a]
                            && t > false_t
                        {
                            return Some(t);
                        }
                        None
                    });

                    if let Some(merge) = else_goto {
                        let merge = merge.min(end); // don't escape our range
                        let then_body_end = *then_addrs.last().unwrap();
                        let then_body =
                            structure_range_d(flat, true_t, then_body_end, depth + 1, targets);
                        let else_body = structure_range_d(flat, false_t, merge, depth + 1, targets);
                        result.push(Stmt::If {
                            cond: cond.clone(),
                            then_body,
                            else_body,
                        });
                        idx = advance_to(&addrs, merge);
                        continue;
                    }

                    // Simple if-then.
                    let then_body = structure_range_d(flat, true_t, false_t, depth + 1, targets);
                    result.push(Stmt::If {
                        cond: cond.clone(),
                        then_body,
                        else_body: vec![],
                    });
                    idx = advance_to(&addrs, false_t);
                } else {
                    // ── false_target is closer (inverted / far-jump case) ──
                    let neg_addrs: Vec<usize> =
                        flat.range(false_t..true_t).map(|(a, _)| *a).collect();

                    // While with inverted condition?
                    let back_goto = neg_addrs.last().and_then(|&a| {
                        if let FlatIR::Goto(t) = flat[&a]
                            && t <= addr
                        {
                            return Some(t);
                        }
                        None
                    });

                    if back_goto.is_some() {
                        let body_last = *neg_addrs.last().unwrap();
                        let body = structure_range_d(flat, false_t, body_last, depth + 1, targets);
                        result.push(Stmt::While {
                            cond: negate_cond(cond),
                            body,
                        });
                        idx = advance_to(&addrs, true_t);
                        continue;
                    }

                    // Inverted if-then (negate condition).
                    let then_body = structure_range_d(flat, false_t, true_t, depth + 1, targets);
                    if !then_body.is_empty() {
                        result.push(Stmt::If {
                            cond: negate_cond(cond),
                            then_body,
                            else_body: vec![],
                        });
                    }
                    idx = advance_to(&addrs, true_t);
                }
            }

            FlatIR::Goto(target) => {
                result.push(Stmt::Goto(*target));
                idx += 1;
            }
            FlatIR::Expr(e) => {
                result.push(Stmt::Expr(e.clone()));
                idx += 1;
            }
            FlatIR::Assign(n, e) => {
                result.push(Stmt::Assign(n.clone(), e.clone()));
                idx += 1;
            }
            FlatIR::Return(e) => {
                result.push(Stmt::Return(e.clone()));
                idx += 1;
            }
        }
    }

    result
}

// ── Type inference (from typed opcodes) ──────────────────────
//
// SCB doesn't store per-slot types for params/vols/tmps, but the opcodes
// are typed: `IAdd` vs `FAdd`, `Aff0Integer` vs `Aff0Float`, the cast
// ops, etc. A single pass over the instruction stream yields a Symbol →
// Ty map that's good enough to annotate vol locals, infer param types
// from `Aff1GetParam`, and pick up the return type from `ReturnVal`.
//
// Unknown stays as `Option::None` → printed as `any`.

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Ty {
    Int,
    Float,
}

fn ty_str(t: Option<Ty>) -> &'static str {
    match t {
        Some(Ty::Int) => "Int",
        Some(Ty::Float) => "Float",
        None => "any",
    }
}

fn infer_sym_types(instructions: &[Instruction], start: usize, end: usize) -> HashMap<Symbol, Ty> {
    use BinaryOp::*;
    use Instruction::*;
    let mut types: HashMap<Symbol, Ty> = HashMap::new();
    // First-wins on conflicts — mixed use would be a miscompile anyway.
    let mut note = |s: Symbol, t: Ty| {
        types.entry(s).or_insert(t);
    };
    let end = end.min(instructions.len());
    for instr in &instructions[start..end] {
        match *instr {
            Aff0Integer { src, dst } => {
                note(src, Ty::Int);
                note(dst, Ty::Int);
            }
            Aff0Float { src, dst } => {
                note(src, Ty::Float);
                note(dst, Ty::Float);
            }
            Aff1CastToInt { src, dst } => {
                note(src, Ty::Float);
                note(dst, Ty::Int);
            }
            Aff1CastToFloat { src, dst } => {
                note(src, Ty::Int);
                note(dst, Ty::Float);
            }
            Aff1IMinus { src, dst } => {
                note(src, Ty::Int);
                note(dst, Ty::Int);
            }
            Aff1FMinus { src, dst } => {
                note(src, Ty::Float);
                note(dst, Ty::Float);
            }
            Aff0IConstant { dst, .. } => note(dst, Ty::Int),
            Aff0FConstant { dst, .. } => note(dst, Ty::Float),
            IfZeroGoto { sym, .. } | IfNotZeroGoto { sym, .. } => note(sym, Ty::Int),
            Binary { op, dst, a, b } => {
                let (operand, result) = match op {
                    IAdd | ISub | IMult | IDiv | IAor | IAand | IAxor | IInfEq | IInf | ISupEq
                    | ISup | INeq | IEq => (Ty::Int, Ty::Int),
                    FAdd | FSub | FMult | FDiv => (Ty::Float, Ty::Float),
                    FInfEq | FInf | FSupEq | FSup | FNeq | FEq => (Ty::Float, Ty::Int),
                };
                note(a, operand);
                note(b, operand);
                note(dst, result);
            }
            _ => {}
        }
    }
    types
}

/// Byte-offset → Ty for incoming params, inferred from `Aff1GetParam`
/// loads (the dst tmp's type propagates back to the param slot).
fn infer_param_types(
    instructions: &[Instruction],
    start: usize,
    end: usize,
    sym_types: &HashMap<Symbol, Ty>,
) -> HashMap<i32, Ty> {
    use Instruction::*;
    let mut out: HashMap<i32, Ty> = HashMap::new();
    let end = end.min(instructions.len());
    for instr in &instructions[start..end] {
        if let Aff1GetParam { dst, param_offset } = *instr
            && let Some(&t) = sym_types.get(&dst)
        {
            out.entry(param_offset).or_insert(t);
        }
    }
    out
}

/// Return-type source: `void` if the function has no return value slot,
/// else the type of the symbol the `ReturnVal` instruction reads (or
/// `any` if we couldn't infer it).
fn return_type_str(
    instructions: &[Instruction],
    start: usize,
    end: usize,
    sym_types: &HashMap<Symbol, Ty>,
    size_of_return_value: i32,
) -> &'static str {
    if size_of_return_value == 0 {
        return "void";
    }
    use Instruction::*;
    let end = end.min(instructions.len());
    for instr in &instructions[start..end] {
        if let ReturnVal { sym } = *instr
            && let Some(&t) = sym_types.get(&sym)
        {
            return match t {
                Ty::Int => "Int",
                Ty::Float => "Float",
            };
        }
    }
    "any"
}

/// Look up a vol local's type by its printed name (`v0`, `v1`, …).
fn vol_type(name: &str, sym_types: &HashMap<Symbol, Ty>) -> Option<Ty> {
    let slot: u16 = name.strip_prefix('v')?.parse().ok()?;
    sym_types.get(&(0x8000 | slot.checked_mul(4)?)).copied()
}

// ── Script lifecycle base classes ────────────────────────────
//
// Each class the engine binds (soldier / scroll / zone / target / …)
// dispatches a fixed set of lifecycle methods via its VMCoreCustom
// vtable. Rather than re-emitting empty `Initialize() {}` /
// `FilterAIEvent() { return 1; }` stubs on every derived class, we emit
// one abstract base per kind at the top of the file and have derived
// classes `extends` it. Methods whose body exactly matches the base's
// default body are elided from the override list.

#[derive(Copy, Clone, PartialEq, Eq)]
enum DefaultBody {
    /// `{}` — void method.
    Empty,
    /// `return 1;` — the canonical "allow / ok" Int return.
    ReturnOne,
}

fn kind_base_name(kind: ScriptKind) -> &'static str {
    match kind {
        ScriptKind::Mission => "MissionScript",
        ScriptKind::Actor => "ActorScript",
        ScriptKind::Scroll => "ScrollScript",
        ScriptKind::Zone => "ZoneScript",
        ScriptKind::Target => "TargetScript",
        ScriptKind::Waypoint => "WaypointScript",
    }
}

/// (method name, default body) for each kind's lifecycle hooks.
fn kind_defaults(kind: ScriptKind) -> &'static [(&'static str, DefaultBody)] {
    match kind {
        ScriptKind::Mission => &[
            ("Initialize", DefaultBody::ReturnOne),
            ("PostInitialize", DefaultBody::Empty),
            ("Hourglass", DefaultBody::ReturnOne),
            ("CheckVictoryCondition", DefaultBody::ReturnOne),
            ("ProcessMessage", DefaultBody::Empty),
            ("Finalize", DefaultBody::Empty),
            ("Briefing", DefaultBody::Empty),
        ],
        ScriptKind::Actor => &[
            ("Initialize", DefaultBody::ReturnOne),
            ("ActionChange", DefaultBody::Empty),
            ("HandleEvent", DefaultBody::Empty),
            ("ProcessMessage", DefaultBody::Empty),
            ("FilterAIEvent", DefaultBody::ReturnOne),
        ],
        ScriptKind::Scroll => &[
            ("Initialize", DefaultBody::Empty),
            ("IsTaken", DefaultBody::ReturnOne),
            ("Hourglass", DefaultBody::Empty),
        ],
        ScriptKind::Zone => &[
            ("Initialize", DefaultBody::ReturnOne),
            ("EnterZone", DefaultBody::ReturnOne),
            ("ExitZone", DefaultBody::ReturnOne),
        ],
        ScriptKind::Target => &[
            ("Initialize", DefaultBody::ReturnOne),
            ("ActivatedByApple", DefaultBody::ReturnOne),
            ("ActivatedByArrow", DefaultBody::ReturnOne),
            ("ActivatedByHand", DefaultBody::ReturnOne),
            ("ActivatedByHeal", DefaultBody::ReturnOne),
            ("ActivatedByLever", DefaultBody::ReturnOne),
            ("ActivatedByMoney", DefaultBody::ReturnOne),
            ("ActivatedBySearch", DefaultBody::ReturnOne),
            ("ActivatedByStone", DefaultBody::ReturnOne),
            ("ActivatedBySword", DefaultBody::ReturnOne),
            ("ActivatedByNet", DefaultBody::ReturnOne),
        ],
        ScriptKind::Waypoint => &[
            ("Initialize", DefaultBody::ReturnOne),
            ("ReachPoint", DefaultBody::ReturnOne),
        ],
    }
}

/// True when `method_name` is a lifecycle hook for `kind` (i.e. listed
/// in `kind_defaults`) AND the body is a trivial stub that the base
/// class already provides: either an empty body, or the base's declared
/// default return.
fn is_default_body(kind: ScriptKind, method_name: &str, body: &[Stmt]) -> bool {
    let Some(default) = kind_defaults(kind)
        .iter()
        .find(|(n, _)| *n == method_name)
        .map(|(_, d)| *d)
    else {
        return false;
    };
    // Empty is always default — a missing override means "use base," so
    // `{}` is equivalent regardless of return type.
    if body.is_empty() {
        return true;
    }
    matches!(
        (default, body),
        (DefaultBody::ReturnOne, [Stmt::Return(Some(Expr::Int(1)))])
    )
}

/// Emit the abstract base class for `kind` — one per file, once, at the
/// top. Signatures follow `known_function_params` so derived overrides
/// line up.
fn emit_base_class(out: &mut String, kind: ScriptKind) {
    let name = kind_base_name(kind);
    let _ = writeln!(out, "abstract class {name} {{");
    for (method, default) in kind_defaults(kind) {
        let params = base_method_params(kind, method);
        let ret = match default {
            DefaultBody::Empty => "void",
            DefaultBody::ReturnOne => "Int",
        };
        let body = match default {
            DefaultBody::Empty => "{}",
            DefaultBody::ReturnOne => "{ return 1; }",
        };
        let _ = writeln!(out, "    {method}({params}): {ret} {body}");
    }
    let _ = writeln!(out, "}}\n");
}

/// Parameter lists for base-class method signatures. These mirror
/// `known_function_params` but without the `this` receiver — bases are
/// only referenced by name, never self-called, so a plain signature is
/// clearer.
fn base_method_params(kind: ScriptKind, method: &str) -> &'static str {
    match (kind, method) {
        (ScriptKind::Mission, "Initialize") => "seed: Int",
        (ScriptKind::Mission, "Hourglass") => "time_seconds: Int",
        (ScriptKind::Mission, "CheckVictoryCondition") => "time_seconds: Int",
        (ScriptKind::Mission, "ProcessMessage") => "message_code: Int, arg1: Int, arg2: Int",
        (ScriptKind::Mission, "Finalize") => "abandoned: Int",
        (_, "Initialize" | "PostInitialize" | "Briefing") => "",
        (ScriptKind::Actor, "ActionChange") => "action: Int, old_action: Int",
        (ScriptKind::Actor, "HandleEvent") => "source_actor: Actor, event_code: Int",
        (ScriptKind::Actor, "ProcessMessage") => "message_code: Int, arg1: Int, arg2: Int",
        (ScriptKind::Actor, "FilterAIEvent") => "source_actor: Actor, event_code: Int",
        (ScriptKind::Scroll, "IsTaken") => "actor: Actor",
        (ScriptKind::Scroll, "Hourglass") => "time_seconds: Int",
        (ScriptKind::Zone, "EnterZone" | "ExitZone") => "actor: Actor",
        (ScriptKind::Target, m) if m.starts_with("ActivatedBy") => "actor: Actor",
        (ScriptKind::Waypoint, "ReachPoint") => "p0: any, p1: any",
        _ => "",
    }
}

/// Render a member variable's declared type as a TypeScript type name.
/// Falls through to `any` for tag values that don't round-trip cleanly
/// (NotDefined, Event, Function, NativeFunction).
fn member_type_str(ty: &robin_engine::scb::ScType) -> String {
    use robin_engine::scb::TypeTag;
    if !ty.native_type_name.is_empty() {
        return ty.native_type_name.clone();
    }
    match ty.tag {
        TypeTag::Int => "Int",
        TypeTag::Float => "Float",
        TypeTag::Bool => "Bool",
        TypeTag::Void => "void",
        _ => "any",
    }
    .to_string()
}

// ── Pretty-printer ───────────────────────────────────────────

/// True for vol-slot locals that `sym_var_name` prints as `v0`, `v1`, …
fn is_vol_local_name(s: &str) -> bool {
    let rest = match s.strip_prefix('v') {
        Some(r) if !r.is_empty() => r,
        _ => return false,
    };
    rest.bytes().all(|b| b.is_ascii_digit())
}

/// Collect every `Stmt::Goto(addr)` target that appears in the tree.
fn collect_stmt_goto_targets(stmts: &[Stmt], out: &mut HashSet<usize>) {
    for stmt in stmts {
        match stmt {
            Stmt::Goto(addr) => {
                out.insert(*addr);
            }
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_stmt_goto_targets(then_body, out);
                collect_stmt_goto_targets(else_body, out);
            }
            Stmt::While { body, .. } => collect_stmt_goto_targets(body, out),
            _ => {}
        }
    }
}

/// Post-structuring cleanup: drop labels with no incoming goto, and drop
/// `Goto(addr); Label(addr);` pairs when the goto lands on the next
/// statement. Live goto targets are collected across the whole tree so
/// a top-level goto can keep a label inside a nested `if`/`while` body
/// alive (and vice-versa).
fn prune_dead_jumps(stmts: &mut Vec<Stmt>) {
    drop_adjacent_goto_label(stmts);

    let mut live: HashSet<usize> = HashSet::new();
    collect_stmt_goto_targets(stmts, &mut live);
    drop_dead_labels(stmts, &live);
}

fn drop_adjacent_goto_label(stmts: &mut Vec<Stmt>) {
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                drop_adjacent_goto_label(then_body);
                drop_adjacent_goto_label(else_body);
            }
            Stmt::While { body, .. } => drop_adjacent_goto_label(body),
            _ => {}
        }
    }
    let mut i = 0;
    while i + 1 < stmts.len() {
        if let (Stmt::Goto(ga), Stmt::Label(la)) = (&stmts[i], &stmts[i + 1])
            && ga == la
        {
            stmts.drain(i..=i + 1);
            continue;
        }
        i += 1;
    }
}

fn drop_dead_labels(stmts: &mut Vec<Stmt>, live: &HashSet<usize>) {
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                drop_dead_labels(then_body, live);
                drop_dead_labels(else_body, live);
            }
            Stmt::While { body, .. } => drop_dead_labels(body, live),
            _ => {}
        }
    }
    stmts.retain(|s| !matches!(s, Stmt::Label(addr) if !live.contains(addr)));
}

fn collect_vol_locals(stmts: &[Stmt], out: &mut BTreeSet<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::Assign(name, _) if is_vol_local_name(name) => {
                out.insert(name.clone());
            }
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_vol_locals(then_body, out);
                collect_vol_locals(else_body, out);
            }
            Stmt::While { body, .. } => collect_vol_locals(body, out),
            _ => {}
        }
    }
}

fn emit_stmts(out: &mut String, stmts: &[Stmt], indent: usize) {
    let pad = "    ".repeat(indent);
    for stmt in stmts {
        match stmt {
            Stmt::Expr(e) => {
                let _ = writeln!(out, "{pad}{};", strip_parens(&e.to_string()));
            }
            Stmt::Assign(name, e) => {
                let _ = writeln!(out, "{pad}{name} = {};", strip_parens(&e.to_string()));
            }
            Stmt::Return(Some(e)) => {
                let _ = writeln!(out, "{pad}return {};", strip_parens(&e.to_string()));
            }
            Stmt::Return(None) => {
                let _ = writeln!(out, "{pad}return;");
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => {
                let _ = writeln!(out, "{pad}if ({}) {{", strip_parens(&cond.to_string()));
                emit_stmts(out, then_body, indent + 1);
                if !else_body.is_empty() {
                    let _ = writeln!(out, "{pad}}} else {{");
                    emit_stmts(out, else_body, indent + 1);
                }
                let _ = writeln!(out, "{pad}}}");
            }
            Stmt::While { cond, body } => {
                let _ = writeln!(out, "{pad}while ({}) {{", strip_parens(&cond.to_string()));
                emit_stmts(out, body, indent + 1);
                let _ = writeln!(out, "{pad}}}");
            }
            Stmt::Goto(addr) => {
                // TypeScript has no `goto`, so emit it as a comment —
                // the label below stays as a valid labelled statement
                // (`g<addr>:`) so tooling / humans can still navigate
                // the dispatch chain.
                let _ = writeln!(out, "{pad}// goto g{addr};");
            }
            Stmt::Label(addr) => {
                let _ = writeln!(out, "{pad}g{addr}:;");
            }
        }
    }
}

// ── Top-level entry points ───────────────────────────────────

/// Decompile an entire `.scb` file to TypeScript-like pseudo-code.
pub fn decompile(scb: &ScbFile) -> String {
    decompile_with_names(scb, None)
}

/// Same, but with an optional actor-name table: when present, rewrite
/// `GetActorScript(/*iPosition*/ N)` → `Actors.Name` and emit an
/// `Actors` const at the top of the file.
pub fn decompile_with_names(scb: &ScbFile, actor_names: Option<&ActorNames>) -> String {
    let mut out = String::new();
    emit_type_prelude(&mut out, scb);

    let class_kind = |class_name: &str| -> Option<ScriptKind> {
        actor_names.and_then(|n| n.kind_of(class_name))
    };

    // Emit every base kind at least one class in this file will extend.
    // Fixed canonical order so the output stays deterministic.
    let used: Vec<ScriptKind> = [
        ScriptKind::Mission,
        ScriptKind::Actor,
        ScriptKind::Scroll,
        ScriptKind::Zone,
        ScriptKind::Target,
        ScriptKind::Waypoint,
    ]
    .into_iter()
    .filter(|&k| {
        scb.classes
            .iter()
            .any(|c| class_kind(&c.class_name) == Some(k))
    })
    .collect();
    for k in &used {
        emit_base_class(&mut out, *k);
    }

    if let Some(names) = actor_names {
        let scroll_texts = build_scroll_popup_text_map(scb, names);
        emit_actors_const(&mut out, names, &scroll_texts);
    }
    for class in &scb.classes {
        decompile_class(&mut out, class, actor_names, class_kind(&class.class_name));
    }
    out
}

fn emit_actors_const(out: &mut String, names: &ActorNames, scroll_texts: &HashMap<usize, String>) {
    // Group the flat `actors` array by slot kind so consumers read
    // `Anim.Nottingham_eau01` instead of `Actors.Anim_Nottingham_eau01`.
    // Emission order matches `ActorSlotKind` declaration order (patch
    // FX, animations, actors, targets, bonuses, scrolls) so the output
    // stays deterministic regardless of HashMap iteration.
    use crate::actor_names::ActorSlotKind;
    let kinds = [
        ActorSlotKind::PatchFx,
        ActorSlotKind::Anim,
        ActorSlotKind::Actor,
        ActorSlotKind::Target,
        ActorSlotKind::Bonus,
        ActorSlotKind::Scroll,
    ];
    for kind in kinds {
        let entries: Vec<(usize, &str)> = names
            .actors
            .iter()
            .zip(names.actor_kinds.iter())
            .enumerate()
            .filter_map(|(i, (name, slot_kind))| {
                let n = name.as_deref()?;
                if *slot_kind != Some(kind) {
                    return None;
                }
                Some((i, n))
            })
            .collect();
        let per_entry_docs = if kind == ActorSlotKind::Scroll {
            Some(scroll_texts)
        } else {
            None
        };
        emit_name_const(
            out,
            kind.global_name(),
            "GetActorScript",
            &entries,
            per_entry_docs,
        );
    }

    let patch_entries: Vec<(usize, &str)> = names
        .patches
        .iter()
        .enumerate()
        .filter_map(|(i, n)| n.as_deref().map(|s| (i, s)))
        .collect();
    emit_name_const(out, "Patches", "GetPatchScript", &patch_entries, None);
}

fn emit_name_const(
    out: &mut String,
    const_name: &str,
    getter: &str,
    entries: &[(usize, &str)],
    per_entry_docs: Option<&HashMap<usize, String>>,
) {
    if entries.is_empty() {
        return;
    }
    let _ = writeln!(out, "const {const_name} = {{");
    for (i, name) in entries {
        if let Some(docs) = per_entry_docs
            && let Some(doc) = docs.get(i)
        {
            let _ = writeln!(out, "    /** {doc} */");
        }
        let _ = writeln!(out, "    {name}: {getter}({i}),");
    }
    let _ = writeln!(out, "}};\n");
}

/// For each `ScrollScript` class, scan for its first
/// `DisplayPopupText(N)` and build `slot_index → formatted doc string`
/// keyed by actor-slot index. `emit_actors_const` drops the string as
/// a doc comment on the matching `Scrolls` entry.
///
/// We match by the sanitized class name embedded in `actor_names.actors`
/// (the same transformation `pick_base_name` applies), stripping any
/// `_N` dedup suffix. That's robust to class-declaration order not
/// matching mission-scroll order — we only need the name round-trip.
fn build_scroll_popup_text_map(scb: &ScbFile, names: &ActorNames) -> HashMap<usize, String> {
    use crate::actor_names::ActorSlotKind;
    // Sanitized class name → popup text id from the class's IsTaken.
    let mut by_sanitized: HashMap<String, i32> = HashMap::new();
    for class in &scb.classes {
        if names.kind_of(&class.class_name) != Some(ScriptKind::Scroll) {
            continue;
        }
        if let Some(id) = find_first_popup_text_id(class) {
            by_sanitized.insert(sanitize_class_name_for_lookup(&class.class_name), id);
        }
    }
    if by_sanitized.is_empty() {
        return HashMap::new();
    }

    let mut out = HashMap::new();
    for (i, (kind, name)) in names
        .actor_kinds
        .iter()
        .zip(names.actors.iter())
        .enumerate()
    {
        if *kind != Some(ActorSlotKind::Scroll) {
            continue;
        }
        let Some(n) = name.as_deref() else { continue };
        let base = strip_dedup_suffix(n);
        let Some(text_id) = by_sanitized.get(base).copied() else {
            continue;
        };
        if let Some(text) = names.popup_text(text_id) {
            out.insert(i, format!("\"{}\"", escape_text_for_comment(text)));
        }
    }
    out
}

/// Walk a class's instructions looking for the first
/// `NativeCall(DisplayPopupText)` and return the int literal pushed
/// immediately before it as the popup-text ID. Returns `None` if the
/// pattern doesn't match (e.g. the ID is computed, or no popup call).
fn find_first_popup_text_id(class: &ClassEntry) -> Option<i32> {
    use robin_engine::vm::Instruction::*;
    let instrs: Vec<_> = class.quads.iter().map(|q| decode(*q).ok()).collect();
    for (i, ins) in instrs.iter().enumerate() {
        let Some(NativeCall { index }) = ins else {
            continue;
        };
        if native_name(*index) != "DisplayPopupText" {
            continue;
        }
        // Walk backwards for the ICONST that set the arg. The
        // emitter puts `ICONST; NATIVEPARAM;` right before the call,
        // so we skip the `NATIVEPARAM` (and stray `NOP`s) to find it.
        for j in (0..i).rev() {
            match &instrs[j] {
                Some(Aff0IConstant { constant, .. }) => return Some(*constant),
                Some(Nop) | Some(NativeParam { .. }) | None => continue,
                _ => break,
            }
        }
    }
    None
}

/// Reverse of `pick_base_name` + `push_with_name` dedup — turns
/// `Kent_2` back into `Kent` for the lookup into `by_sanitized`.
fn strip_dedup_suffix(name: &str) -> &str {
    if let Some((head, tail)) = name.rsplit_once('_')
        && !tail.is_empty()
        && tail.bytes().all(|b| b.is_ascii_digit())
    {
        head
    } else {
        name
    }
}

/// Mirror of `actor_names::sanitize_class_name` — reproduced here so
/// we don't need to expose it from that module just for this lookup.
fn sanitize_class_name_for_lookup(class: &str) -> String {
    let trimmed = match class.rsplit_once('_') {
        Some((head, tail)) if tail.len() == 8 && tail.chars().all(|c| c.is_ascii_hexdigit()) => {
            head
        }
        _ => class,
    };
    let mut out = String::with_capacity(trimmed.len());
    let mut last_underscore = false;
    for c in trimmed.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    out.trim_matches('_').to_owned()
}

/// Header emitted once per file: alias every "semantic" type seen in
/// class member declarations (plus the primitives we infer) to `number`,
/// which is what they actually are at runtime. Lets the output parse as
/// real TypeScript without us hand-maintaining a schema.
fn emit_type_prelude(out: &mut String, scb: &ScbFile) {
    let mut aliases: BTreeSet<String> = BTreeSet::new();
    // Primitives from inference + tag-typed members.
    aliases.insert("Int".to_string());
    aliases.insert("Float".to_string());
    aliases.insert("Bool".to_string());
    // Native handle types pulled from every class's members.
    for class in &scb.classes {
        for mv in &class.member_variables {
            if !mv.ty.native_type_name.is_empty() {
                aliases.insert(mv.ty.native_type_name.clone());
            }
        }
    }
    for name in &aliases {
        let _ = writeln!(out, "type {name} = number;");
    }
    let _ = writeln!(out);
}

/// Walk a stmt tree and rewrite `GetActorScript(N)` calls to
/// `Actors.Name` references where a name is known. Also pokes through the
/// `NamedArg` wrapper so `/*actor*/ GetActorScript(N)` becomes
/// `/*actor*/ Actors.Name` (the comment stays useful when the slot is
/// known but the NamedArg annotation is preserved).
fn rewrite_actor_refs(stmts: &mut [Stmt], names: &ActorNames) {
    for stmt in stmts {
        match stmt {
            Stmt::Expr(e) | Stmt::Assign(_, e) => rewrite_expr(e, names),
            Stmt::Return(Some(e)) => rewrite_expr(e, names),
            Stmt::Return(None) | Stmt::Goto(_) | Stmt::Label(_) => {}
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => {
                rewrite_expr(cond, names);
                rewrite_actor_refs(then_body, names);
                rewrite_actor_refs(else_body, names);
            }
            Stmt::While { cond, body } => {
                rewrite_expr(cond, names);
                rewrite_actor_refs(body, names);
            }
        }
    }
}

fn rewrite_expr(e: &mut Expr, names: &ActorNames) {
    // Post-order: recurse first, then try to rewrite at this node.
    match e {
        Expr::Call(_, args) => {
            for a in args.iter_mut() {
                rewrite_expr(a, names);
            }
        }
        Expr::BinOp(_, l, r) => {
            rewrite_expr(l, names);
            rewrite_expr(r, names);
        }
        Expr::Neg(inner) | Expr::Cast(_, inner) => rewrite_expr(inner, names),
        Expr::NamedArg(_, inner) => rewrite_expr(inner, names),
        Expr::WithTrailingComment(inner, _) => rewrite_expr(inner, names),
        Expr::Int(_) | Expr::Float(_) | Expr::Var(_) => {}
    }
    // `Get*Script(<int literal>)` → `<Global>.Name` when known. The
    // global name is the actor's slot kind (`Actors`, `Anim`, `PatchFx`,
    // …) for `GetActorScript`; always `Patches` for `GetPatchScript`.
    if let Expr::Call(fn_name, args) = e
        && args.len() == 1
        && let Some(pos) = as_int_literal(&args[0])
    {
        let rewrite = match fn_name.as_str() {
            "GetActorScript" => names.actor_qualified(pos).map(|(g, n)| format!("{g}.{n}")),
            "GetPatchScript" => names.patch(pos).map(|n| format!("Patches.{n}")),
            _ => None,
        };
        if let Some(ident) = rewrite {
            *e = Expr::Var(ident);
        }
    }
    // Drop the `/*paramname*/` prefix when the argument resolved to one
    // of our named globals (Actors/Anim/PatchFx/Targets/
    // Bonuses/Scrolls/Patches): the identifier itself is more
    // informative than the slot name.
    if let Expr::NamedArg(_, inner) = e
        && is_resolved_global(inner)
    {
        let taken = std::mem::replace(inner.as_mut(), Expr::Int(0));
        *e = taken;
    }
}

fn is_resolved_global(e: &Expr) -> bool {
    const PREFIXES: &[&str] = &[
        "Actors.", "Anim.", "PatchFx.", "Targets.", "Bonuses.", "Scrolls.", "Patches.",
    ];
    match e {
        Expr::Var(name) => PREFIXES.iter().any(|p| name.starts_with(p)),
        _ => false,
    }
}

fn as_int_literal(e: &Expr) -> Option<i32> {
    match e {
        Expr::Int(n) => Some(*n),
        Expr::NamedArg(_, inner) | Expr::WithTrailingComment(inner, _) => as_int_literal(inner),
        _ => None,
    }
}

/// Annotate string-table-index arguments on calls like `DisplayPopupText`
/// and `DoneShortBriefing` with the actual text as a trailing comment, so
/// readers see what the scroll/briefing says without cross-referencing
/// the `.res` file by hand.
fn annotate_text_ids(stmts: &mut [Stmt], names: &ActorNames) {
    for stmt in stmts {
        match stmt {
            Stmt::Expr(e) | Stmt::Assign(_, e) => annotate_text_in_expr(e, names),
            Stmt::Return(Some(e)) => annotate_text_in_expr(e, names),
            Stmt::Return(None) | Stmt::Goto(_) | Stmt::Label(_) => {}
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => {
                annotate_text_in_expr(cond, names);
                annotate_text_ids(then_body, names);
                annotate_text_ids(else_body, names);
            }
            Stmt::While { cond, body } => {
                annotate_text_in_expr(cond, names);
                annotate_text_ids(body, names);
            }
        }
    }
}

fn annotate_text_in_expr(e: &mut Expr, names: &ActorNames) {
    match e {
        Expr::Call(fn_name, args) => {
            let lookup: Option<fn(&ActorNames, i32) -> Option<&str>> = match fn_name.as_str() {
                "DisplayPopupText" => Some(|n, id| n.popup_text(id)),
                "DoneShortBriefing" => Some(|n, id| n.short_briefing_text(id)),
                _ => None,
            };
            if let Some(get_text) = lookup
                && let Some(first) = args.first_mut()
                && let Some(id) = as_int_literal(first)
                && let Some(text) = get_text(names, id)
            {
                let escaped = escape_text_for_comment(text);
                let taken = std::mem::replace(first, Expr::Int(0));
                *first = Expr::WithTrailingComment(Box::new(taken), format!("\"{escaped}\""));
            }
            for a in args.iter_mut() {
                annotate_text_in_expr(a, names);
            }
        }
        Expr::BinOp(_, l, r) => {
            annotate_text_in_expr(l, names);
            annotate_text_in_expr(r, names);
        }
        Expr::Neg(inner) | Expr::Cast(_, inner) => annotate_text_in_expr(inner, names),
        Expr::NamedArg(_, inner) | Expr::WithTrailingComment(inner, _) => {
            annotate_text_in_expr(inner, names)
        }
        Expr::Int(_) | Expr::Float(_) | Expr::Var(_) => {}
    }
}

/// Make `text` safe to drop inside `/* … */`: strip control chars,
/// collapse newlines to `\n`, and close/open any `*/` sequence that
/// would terminate the comment early.
fn escape_text_for_comment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '*' if chars.peek() == Some(&'/') => {
                out.push_str("*\\/");
                chars.next();
            }
            '"' => out.push_str("\\\""),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

/// Rewrite `NamedArg("bFoo", Int(0|1))` → the TS literal `true`/`false`.
/// Pattern match on the lowerCamel `b` prefix: `b` followed by an
/// uppercase ASCII letter (e.g. `bState`, `bYes`, `bRememberEvents`,
/// `bTrueMeansForbidFalseMeansAllow`).
fn rewrite_bool_args(stmts: &mut [Stmt]) {
    for stmt in stmts {
        match stmt {
            Stmt::Expr(e) | Stmt::Assign(_, e) => rewrite_bool_expr(e),
            Stmt::Return(Some(e)) => rewrite_bool_expr(e),
            Stmt::Return(None) | Stmt::Goto(_) | Stmt::Label(_) => {}
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => {
                rewrite_bool_expr(cond);
                rewrite_bool_args(then_body);
                rewrite_bool_args(else_body);
            }
            Stmt::While { cond, body } => {
                rewrite_bool_expr(cond);
                rewrite_bool_args(body);
            }
        }
    }
}

fn rewrite_bool_expr(e: &mut Expr) {
    match e {
        Expr::Call(_, args) => {
            for a in args.iter_mut() {
                rewrite_bool_expr(a);
            }
        }
        Expr::BinOp(_, l, r) => {
            rewrite_bool_expr(l);
            rewrite_bool_expr(r);
        }
        Expr::Neg(inner) | Expr::Cast(_, inner) => rewrite_bool_expr(inner),
        Expr::NamedArg(name, inner) => {
            rewrite_bool_expr(inner);
            if is_bool_param_name(name)
                && let Expr::Int(n) = **inner
                && (n == 0 || n == 1)
            {
                **inner = Expr::Var(if n == 0 { "false" } else { "true" }.to_owned());
            }
        }
        Expr::WithTrailingComment(inner, _) => rewrite_bool_expr(inner),
        Expr::Int(_) | Expr::Float(_) | Expr::Var(_) => {}
    }
}

fn is_bool_param_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next() == Some('b') && chars.next().is_some_and(|c| c.is_ascii_uppercase())
}

fn decompile_class(
    out: &mut String,
    class: &ClassEntry,
    actor_names: Option<&ActorNames>,
    kind: Option<ScriptKind>,
) {
    let extends = kind
        .map(|k| format!(" extends {}", kind_base_name(k)))
        .unwrap_or_default();
    let _ = writeln!(out, "class {}{} {{", class.class_name, extends);

    // Members
    if !class.member_variables.is_empty() {
        for mv in &class.member_variables {
            let _ = writeln!(out, "    {}: {};", mv.name, member_type_str(&mv.ty));
        }
        let _ = writeln!(out);
    }

    // Build lookup maps
    let member_map: HashMap<usize, &str> = class
        .member_variables
        .iter()
        .map(|mv| (mv.address as usize, mv.name.as_str()))
        .collect();
    let func_map: HashMap<usize, &str> = class
        .functions
        .iter()
        .map(|f| (f.address as usize, f.name.as_str()))
        .collect();

    // Decode all instructions
    let instructions: Vec<Instruction> = class
        .quads
        .iter()
        .map(|q| decode(*q).unwrap_or(Instruction::Empty))
        .collect();

    // Decompile each function
    for (fi, func) in class.functions.iter().enumerate() {
        let start = func.address as usize;
        let end = if fi + 1 < class.functions.len() {
            class.functions[fi + 1].address as usize
        } else {
            instructions.len()
        };

        // Function signature with known parameter names
        let param_count = func.size_of_parameters as usize / 4;
        let known = known_function_params(&class.class_name, &func.name, param_count);
        if let Some(ref names) = known
            && names.len() != param_count
        {
            tracing::warn!(
                "Decompiler: {}::{} has {} params in bytecode but {} known names",
                class.class_name,
                func.name,
                param_count,
                names.len(),
            );
        }
        let params: Vec<String> = (0..param_count)
            .map(|i| {
                known
                    .as_ref()
                    .and_then(|names| names.get(i))
                    .map(|&s| s.to_owned())
                    .unwrap_or_else(|| format!("p{i}"))
            })
            .collect();

        // Types come from a single forward pass over the function body.
        let sym_types = infer_sym_types(&instructions, start, end);
        let param_types = infer_param_types(&instructions, start, end, &sym_types);
        let ret_ty = return_type_str(
            &instructions,
            start,
            end,
            &sym_types,
            func.size_of_return_value,
        );

        // Body pipeline: fold → structure → rewrite → prune.
        let flat = fold_expressions(&instructions, start, end, &member_map, &func_map, &params);
        let mut stmts = structure_range(&flat, start, end);
        if let Some(n) = actor_names {
            rewrite_actor_refs(&mut stmts, n);
            annotate_text_ids(&mut stmts, n);
        }
        prune_dead_jumps(&mut stmts);
        rewrite_bool_args(&mut stmts);

        // Elide overrides that only repeat the base-class default body.
        if let Some(k) = kind
            && is_default_body(k, &func.name, &stmts)
        {
            continue;
        }

        // TS-style signature with per-param type annotations.
        let typed_params: Vec<String> = params
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let param_offset = (i * 4) as i32;
                let pty = if name == "this" {
                    class.class_name.clone()
                } else {
                    ty_str(param_types.get(&param_offset).copied()).to_string()
                };
                format!("{name}: {pty}")
            })
            .collect();
        let _ = writeln!(
            out,
            "    {}({}): {ret_ty} {{",
            func.name,
            typed_params.join(", "),
        );

        // Hoist vol locals into a single `let` at function top — the bytecode
        // scopes `v0..vN` to the whole frame, so first-assignment inside an
        // `if` branch must not become a block-scoped `let` in the output.
        let mut locals: BTreeSet<String> = BTreeSet::new();
        collect_vol_locals(&stmts, &mut locals);
        if !locals.is_empty() {
            let typed: Vec<String> = locals
                .iter()
                .map(|n| format!("{n}: {}", ty_str(vol_type(n, &sym_types))))
                .collect();
            let _ = writeln!(out, "        let {};", typed.join(", "));
        }

        emit_stmts(out, &stmts, 2);
        let _ = writeln!(out, "    }}\n");
    }

    let _ = writeln!(out, "}}\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scb;

    #[test]
    fn decompile_demo_script() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let base = std::path::PathBuf::from(manifest_dir).join("../../datadirs");
        let scb_name = "Data/Levels/Dem_Lei_MP.scb";
        let path = base.join("demo_ecoste").join(scb_name);
        let path = if path.exists() {
            path
        } else {
            let alt = base.join("demo").join(scb_name);
            if !alt.exists() {
                return; // skip if no datadirs
            }
            alt
        };
        let scb = scb::parse_file(&path).unwrap();
        let text = decompile(&scb);

        // Basic structure present
        assert!(text.contains("class StartUp {"));
        assert!(text.contains("PutActorInBuilding()"));
        assert!(text.contains("Initialize("));

        // Expression folding: constants inlined into calls
        assert!(text.contains("GetActorScript(136)"));
        assert!(text.contains("GetBuildingScript(1)"));

        // Member variable naming
        assert!(text.contains("this.locWill"));
        assert!(text.contains("this.iOldSeconds1"));

        // If-then recovery
        assert!(text.contains("if ("));

        // While loop recovery
        assert!(text.contains("while ("));

        // No raw tmp/sym references — everything should be folded
        assert!(!text.contains("t0 ="), "leftover tmp assignment in output");

        // Sanity-check output size (the switch-dispatch bug produced 1.4M lines).
        let lines = text.lines().count();
        assert!(
            lines < 5000,
            "output too large: {lines} lines — likely a structuring bug"
        );
    }
}
