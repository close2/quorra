//! The §7.10.5 PostScript calculator, compiled to a flat instruction list.
//!
//! This is the *spike's* stand-in for the compiled form the caller offers to hand us
//! (`QUORRA_FUNCTION_PAINT.md` §4: "we would hand you the compiled form, not
//! PostScript text"). It exists here only so the spike has something to measure; the
//! lexical work — comments, tokens, `{}` matching — stays theirs if this is ever
//! built for real.
//!
//! Two properties are asserted by construction, because the whole feasibility
//! argument rests on them:
//!
//! - **Every jump is forward.** [`Program::verify_forward_only`] checks it, and it is
//!   what makes the instruction count a bound on the execution: a `for` loop of
//!   `op_count` iterations in WGSL cannot be exceeded by any program.
//! - **The stack depth is known before the frame**, by the same symbolic walk that
//!   the generated shader uses to name its slots (`walk::analyse`).
//!
//! Types: ISO 32000-2 §7.10.5.1 gives the calculator integers, reals and booleans,
//! and Table 42's `and`, `or`, `xor` and `not` are *type-sensitive* — bitwise on
//! integers, logical on booleans. A value therefore carries a [`Kind`], but only
//! while it is being compiled: `walk::analyse` infers the kind of every stack slot
//! statically and rewrites `not` to whichever of the two it meant, so neither shader
//! carries a type tag at run time. `and`, `or` and `xor` need no rewrite — with
//! `true` as 1 and `false` as 0, the bitwise reading and the logical one agree.

use std::fmt::Write as _;

use crate::refusal::Refusal;

/// What a value on the operand stack is, in the sense ISO 32000-2 §7.10.5.1 gives
/// the three types.
///
/// The lattice matters at an `if`/`ifelse` merge: two branches that leave different
/// kinds in the same slot leave a value whose kind is only known at run time, and the
/// generated shader has no run-time kinds. See [`Kind::merge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A real number.
    Real,
    /// An integer.
    Int,
    /// `true` or `false`.
    Bool,
    /// Two branches disagreed and no static kind describes the value.
    Unknown,
}

impl Kind {
    /// The kind a slot has after two branches of an `ifelse` wrote different kinds.
    ///
    /// `Int` and `Real` merge to `Real`: every operator that accepts a real accepts
    /// an integer, so the merge loses nothing a real consumer needs. `Bool` merged
    /// with anything else is `Unknown`, because `not` is the one operator whose
    /// meaning genuinely differs (logical negation against a one's complement).
    pub(crate) fn merge(self, other: Self) -> Self {
        match (self, other) {
            (a, b) if a == b => a,
            (Self::Int, Self::Real) | (Self::Real, Self::Int) => Self::Real,
            _ => Self::Unknown,
        }
    }
}

/// One instruction of the flat list.
///
/// The operator set is ISO 32000-2 Table 42 exactly, minus `if`/`ifelse` — which are
/// control flow and become [`Op::JumpIfFalse`] and [`Op::Jump`] — and plus the three
/// literal pushes Table 42 has no name for because PostScript spells them as tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u32)]
pub(crate) enum Op {
    /// Push a real literal.
    PushReal(f32) = 0,
    /// Push an integer literal.
    PushInt(i32) = 1,
    /// Push `true` or `false`.
    PushBool(bool) = 2,

    // §7.10.5.2, "Arithmetic operators".
    /// `abs`.
    Abs = 3,
    /// `add`.
    Add = 4,
    /// `atan` — degrees, in `[0, 360)`.
    Atan = 5,
    /// `ceiling`.
    Ceiling = 6,
    /// `cos` — argument in degrees.
    Cos = 7,
    /// `cvi` — convert to integer, truncating toward zero.
    Cvi = 8,
    /// `cvr` — convert to real.
    Cvr = 9,
    /// `div` — real division.
    Div = 10,
    /// `exp` — `base exponent exp`.
    Exp = 11,
    /// `floor`.
    Floor = 12,
    /// `idiv` — integer division, truncating.
    IDiv = 13,
    /// `ln` — natural logarithm.
    Ln = 14,
    /// `log` — base-10 logarithm.
    Log = 15,
    /// `mod` — integer remainder.
    Mod = 16,
    /// `mul`.
    Mul = 17,
    /// `neg`.
    Neg = 18,
    /// `round`.
    Round = 19,
    /// `sin` — argument in degrees.
    Sin = 20,
    /// `sqrt`.
    Sqrt = 21,
    /// `sub`.
    Sub = 22,
    /// `truncate`.
    Truncate = 23,

    // §7.10.5.2, "Relational, boolean and bitwise operators".
    /// `and` — bitwise on integers, logical on booleans.
    And = 24,
    /// `bitshift` — left for positive shifts, right for negative.
    BitShift = 25,
    /// `eq`.
    Eq = 26,
    /// `ge`.
    Ge = 27,
    /// `gt`.
    Gt = 28,
    /// `le`.
    Le = 29,
    /// `lt`.
    Lt = 30,
    /// `ne`.
    Ne = 31,
    /// `not` — logical on booleans, one's complement on integers.
    Not = 32,
    /// `or`.
    Or = 33,
    /// `xor`.
    Xor = 34,

    // §7.10.5.2, "Stack operators".
    /// `copy` — duplicate the top *n*.
    Copy = 35,
    /// `dup`.
    Dup = 36,
    /// `exch`.
    Exch = 37,
    /// `index` — copy the *n*-th element counting down from the top.
    Index = 38,
    /// `pop`.
    Pop = 39,
    /// `roll` — rotate the top *n* by *j*.
    Roll = 40,

    // Control flow. `if` and `ifelse` compile to these; nothing else produces them.
    /// Pop a boolean; jump to the target when it is false.
    JumpIfFalse(u32) = 41,
    /// Jump to the target unconditionally.
    Jump(u32) = 42,

    /// One's complement. Table 42 spells this `not` too; which of the two `not`
    /// means is decided by the operand's type, and [`crate::walk`] resolves it before
    /// either shader sees the list. See [`Op::Not`], which is the logical one.
    BitNot = 43,
}

impl Op {
    /// The opcode the shaders switch on.
    ///
    /// Taken from the discriminant so the WGSL constants and this enum cannot drift:
    /// `program.wgsl`'s `OP_*` constants are generated from
    /// [`wgsl_opcode_constants`].
    pub(crate) fn code(self) -> u32 {
        // The discriminant is explicit above and `#[repr(u32)]` puts it first; this
        // is the documented way to read it without `unsafe`.
        match self {
            Self::PushReal(_) => 0,
            Self::PushInt(_) => 1,
            Self::PushBool(_) => 2,
            Self::Abs => 3,
            Self::Add => 4,
            Self::Atan => 5,
            Self::Ceiling => 6,
            Self::Cos => 7,
            Self::Cvi => 8,
            Self::Cvr => 9,
            Self::Div => 10,
            Self::Exp => 11,
            Self::Floor => 12,
            Self::IDiv => 13,
            Self::Ln => 14,
            Self::Log => 15,
            Self::Mod => 16,
            Self::Mul => 17,
            Self::Neg => 18,
            Self::Round => 19,
            Self::Sin => 20,
            Self::Sqrt => 21,
            Self::Sub => 22,
            Self::Truncate => 23,
            Self::And => 24,
            Self::BitShift => 25,
            Self::Eq => 26,
            Self::Ge => 27,
            Self::Gt => 28,
            Self::Le => 29,
            Self::Lt => 30,
            Self::Ne => 31,
            Self::Not => 32,
            Self::Or => 33,
            Self::Xor => 34,
            Self::Copy => 35,
            Self::Dup => 36,
            Self::Exch => 37,
            Self::Index => 38,
            Self::Pop => 39,
            Self::Roll => 40,
            Self::JumpIfFalse(_) => 41,
            Self::Jump(_) => 42,
            Self::BitNot => 43,
        }
    }

    /// The instruction's immediate, as the 32 bits the shader reads.
    pub(crate) fn operand(self) -> u32 {
        match self {
            Self::PushReal(v) => v.to_bits(),
            Self::PushInt(v) => v.cast_unsigned(),
            Self::PushBool(v) => u32::from(v),
            Self::JumpIfFalse(t) | Self::Jump(t) => t,
            _ => 0,
        }
    }
}

/// A compiled §7.10.5 program: a flat list with forward-only jumps.
///
/// What it costs to *run* — the stack depth, whether it underflows, which `not` is
/// which — is `walk::analyse`'s answer, not this type's.
#[derive(Debug, Clone)]
pub(crate) struct Program {
    /// The instruction list, in execution order.
    pub(crate) ops: Vec<Op>,
    /// How many values the program leaves — the shading's colour components.
    pub(crate) outputs: usize,
    /// How many it consumes: 2 for §8.7.4.5.2's type 1 shading.
    pub(crate) inputs: usize,
}

impl Program {
    /// Every jump target is strictly greater than the jump's own address.
    ///
    /// This is the caller's load-bearing claim (§4: "a flat instruction list with
    /// **forward-only jumps**, so its length bounds its own execution") and the reason
    /// the interpreter's WGSL loop can be a `for` over `op_count` iterations.
    pub(crate) fn verify_forward_only(&self) -> bool {
        self.ops.iter().enumerate().all(|(pc, op)| match op {
            Op::JumpIfFalse(t) | Op::Jump(t) => *t as usize > pc,
            _ => true,
        })
    }

    /// The instruction list as the `(opcode, operand)` pairs the interpreter reads.
    pub(crate) fn wire(&self) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.ops.len() * 2);
        for op in &self.ops {
            out.push(op.code());
            out.push(op.operand());
        }
        out
    }
}

/// One parsed item: an operator, or a `{...}` procedure awaiting its conditional.
enum Item {
    Leaf(Op),
    Proc(Vec<Item>),
}

/// Compile PostScript calculator source to the flat list.
///
/// The source is the text between (and including) the outermost `{` `}`, as it
/// appears in a type 4 function's stream. Comments (`%` to end of line) are stripped.
///
/// # Errors
///
/// A [`Refusal`] naming the token or the structure that could not be compiled.
pub(crate) fn compile(source: &str, inputs: usize, outputs: usize) -> Result<Program, Refusal> {
    let tokens = tokenize(source);
    let mut cursor = 0;
    // A leading `{` is the function's own procedure wrapper; strip it and its mate.
    let body = if tokens.first().map(String::as_str) == Some("{") {
        cursor = 1;
        let items = parse_block(&tokens, &mut cursor)?;
        if cursor != tokens.len() {
            return Err(Refusal::UnbalancedBraces);
        }
        items
    } else {
        parse_block_to_end(&tokens, &mut cursor)?
    };

    let mut ops = Vec::new();
    emit(&body, &mut ops)?;
    Ok(Program {
        ops,
        outputs,
        inputs,
    })
}

/// Split on whitespace and braces, dropping `%` comments.
fn tokenize(source: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_comment = false;
    for ch in source.chars() {
        if in_comment {
            if ch == '\n' {
                in_comment = false;
            }
            continue;
        }
        match ch {
            '%' => {
                in_comment = true;
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            '{' | '}' => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                tokens.push(ch.to_string());
            }
            c if c.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Parse until the matching `}`, which is consumed.
fn parse_block(tokens: &[String], cursor: &mut usize) -> Result<Vec<Item>, Refusal> {
    let mut items = Vec::new();
    loop {
        let Some(token) = tokens.get(*cursor) else {
            return Err(Refusal::UnbalancedBraces);
        };
        *cursor += 1;
        match token.as_str() {
            "}" => return Ok(items),
            "{" => items.push(Item::Proc(parse_block(tokens, cursor)?)),
            other => items.push(Item::Leaf(leaf(other)?)),
        }
    }
}

/// Parse a source with no outer braces, to the end of the token list.
fn parse_block_to_end(tokens: &[String], cursor: &mut usize) -> Result<Vec<Item>, Refusal> {
    let mut items = Vec::new();
    while let Some(token) = tokens.get(*cursor) {
        *cursor += 1;
        match token.as_str() {
            "}" => return Err(Refusal::UnbalancedBraces),
            "{" => items.push(Item::Proc(parse_block(tokens, cursor)?)),
            other => items.push(Item::Leaf(leaf(other)?)),
        }
    }
    Ok(items)
}

/// One token to one instruction. `if`/`ifelse` are handled by [`emit`], not here.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per ISO 32000-2 Table 42 operator; splitting the table hides it"
)]
fn leaf(token: &str) -> Result<Op, Refusal> {
    Ok(match token {
        "abs" => Op::Abs,
        "add" => Op::Add,
        "atan" => Op::Atan,
        "ceiling" => Op::Ceiling,
        "cos" => Op::Cos,
        "cvi" => Op::Cvi,
        "cvr" => Op::Cvr,
        "div" => Op::Div,
        "exp" => Op::Exp,
        "floor" => Op::Floor,
        "idiv" => Op::IDiv,
        "ln" => Op::Ln,
        "log" => Op::Log,
        "mod" => Op::Mod,
        "mul" => Op::Mul,
        "neg" => Op::Neg,
        "round" => Op::Round,
        "sin" => Op::Sin,
        "sqrt" => Op::Sqrt,
        "sub" => Op::Sub,
        "truncate" => Op::Truncate,
        "and" => Op::And,
        "bitshift" => Op::BitShift,
        "eq" => Op::Eq,
        "false" => Op::PushBool(false),
        "ge" => Op::Ge,
        "gt" => Op::Gt,
        "le" => Op::Le,
        "lt" => Op::Lt,
        "ne" => Op::Ne,
        "not" => Op::Not,
        "or" => Op::Or,
        "true" => Op::PushBool(true),
        "xor" => Op::Xor,
        "copy" => Op::Copy,
        "dup" => Op::Dup,
        "exch" => Op::Exch,
        "index" => Op::Index,
        "pop" => Op::Pop,
        "roll" => Op::Roll,
        // `if` and `ifelse` reach `emit` as these markers and are rewritten there.
        "if" => IF_MARKER,
        "ifelse" => IFELSE_MARKER,
        other => {
            if let Ok(i) = other.parse::<i32>() {
                Op::PushInt(i)
            } else if let Ok(r) = other.parse::<f32>() {
                Op::PushReal(r)
            } else {
                return Err(Refusal::UnknownOperator(other.to_string()));
            }
        }
    })
}

/// Marker discriminants for the two conditionals as they arrive from [`leaf`].
const IF_MARKER: Op = Op::JumpIfFalse(u32::MAX);
const IFELSE_MARKER: Op = Op::Jump(u32::MAX);

/// Flatten the item tree, turning `if`/`ifelse` into forward jumps.
fn emit(items: &[Item], ops: &mut Vec<Op>) -> Result<(), Refusal> {
    let mut index = 0;
    while index < items.len() {
        match &items[index] {
            Item::Leaf(op) if *op == IF_MARKER || *op == IFELSE_MARKER => {
                return Err(Refusal::MalformedConditional);
            }
            Item::Leaf(op) => {
                ops.push(*op);
                index += 1;
            }
            Item::Proc(then_body) => {
                index = emit_conditional(items, index, then_body, ops)?;
            }
        }
    }
    Ok(())
}

/// Emit `{a} if` or `{a} {b} ifelse` starting at `index`; returns the next index.
fn emit_conditional(
    items: &[Item],
    index: usize,
    then_body: &[Item],
    ops: &mut Vec<Op>,
) -> Result<usize, Refusal> {
    match items.get(index + 1) {
        Some(Item::Leaf(op)) if *op == IF_MARKER => {
            let patch = ops.len();
            ops.push(Op::JumpIfFalse(0));
            emit(then_body, ops)?;
            let target = u32::try_from(ops.len()).map_err(|_| Refusal::UnbalancedBraces)?;
            ops[patch] = Op::JumpIfFalse(target);
            Ok(index + 2)
        }
        Some(Item::Proc(else_body)) => {
            if !matches!(items.get(index + 2), Some(Item::Leaf(op)) if *op == IFELSE_MARKER) {
                return Err(Refusal::StrayProcedure);
            }
            let branch = ops.len();
            ops.push(Op::JumpIfFalse(0));
            emit(then_body, ops)?;
            let skip = ops.len();
            ops.push(Op::Jump(0));
            let else_start = u32::try_from(ops.len()).map_err(|_| Refusal::UnbalancedBraces)?;
            ops[branch] = Op::JumpIfFalse(else_start);
            emit(else_body, ops)?;
            let end = u32::try_from(ops.len()).map_err(|_| Refusal::UnbalancedBraces)?;
            ops[skip] = Op::Jump(end);
            Ok(index + 3)
        }
        _ => Err(Refusal::StrayProcedure),
    }
}

/// The `OP_*` constants for the interpreter's WGSL, generated from this enum so the
/// two cannot drift.
pub(crate) fn wgsl_opcode_constants() -> String {
    let names = [
        ("PUSH_REAL", 0),
        ("PUSH_INT", 1),
        ("PUSH_BOOL", 2),
        ("ABS", 3),
        ("ADD", 4),
        ("ATAN", 5),
        ("CEILING", 6),
        ("COS", 7),
        ("CVI", 8),
        ("CVR", 9),
        ("DIV", 10),
        ("EXP", 11),
        ("FLOOR", 12),
        ("IDIV", 13),
        ("LN", 14),
        ("LOG", 15),
        ("MOD", 16),
        ("MUL", 17),
        ("NEG", 18),
        ("ROUND", 19),
        ("SIN", 20),
        ("SQRT", 21),
        ("SUB", 22),
        ("TRUNCATE", 23),
        ("AND", 24),
        ("BITSHIFT", 25),
        ("EQ", 26),
        ("GE", 27),
        ("GT", 28),
        ("LE", 29),
        ("LT", 30),
        ("NE", 31),
        ("NOT", 32),
        ("OR", 33),
        ("XOR", 34),
        ("COPY", 35),
        ("DUP", 36),
        ("EXCH", 37),
        ("INDEX", 38),
        ("POP", 39),
        ("ROLL", 40),
        ("JUMP_IF_FALSE", 41),
        ("JUMP", 42),
        ("BIT_NOT", 43),
    ];
    let mut out = String::new();
    for (name, code) in names {
        let _ = writeln!(out, "const OP_{name}: u32 = {code}u;");
    }
    out
}
