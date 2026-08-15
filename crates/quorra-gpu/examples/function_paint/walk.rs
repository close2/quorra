//! The one symbolic walk, which answers both shapes' questions.
//!
//! Walking the flat list with a *compile-time* operand stack gives, in one pass:
//!
//! - the greatest depth the program reaches, so the interpreter's array can be sized
//!   and a deeper program refused before the frame rather than clipped during it;
//! - which of Table 42's two `not`s each occurrence means, so neither shader needs a
//!   run-time type tag;
//! - whether the program pops an empty stack, which the caller's own witness does;
//! - and, in [`Mode::Generate`], the WGSL for shape (ii) — where the stack position of
//!   every value is a *name*, `s0`..`sN`, so nothing is dynamically indexed and the
//!   whole operand stack lives in registers.
//!
//! That last point is the whole reason to expect shape (ii) to win: the interpreter's
//! `stack[sp]` is an index no compiler can resolve, and on every GPU that means
//! scratch memory.
//!
//! The walk is possible at all because the jumps are forward-only and structured —
//! each `JumpIfFalse` is either a plain `if` or is paired with the `Jump` that ends
//! its then-arm — so the list reads as a tree without a control-flow analysis.

use std::fmt::Write as _;

use crate::program::{Kind, Op, Program};
use crate::refusal::Refusal;

/// What the walk is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Facts only: depth, `not` resolution, underflow. What shape (i) needs.
    Analyse,
    /// Facts and the WGSL. What shape (ii) needs, and it refuses more.
    Generate,
}

/// What the walk found.
#[derive(Debug, Clone)]
pub(crate) struct Facts {
    /// The instruction list with each `not` resolved to the operator it meant.
    pub(crate) ops: Vec<Op>,
    /// The greatest operand-stack depth reached, inputs included.
    pub(crate) max_depth: usize,
    /// How many times the program pops a value that is not there.
    pub(crate) underflows: usize,
    /// How many `if`/`ifelse` the program contains.
    pub(crate) branches: usize,
    /// Instructions whose result does not depend on the fragment's coordinate, and
    /// which a shader compiler therefore evaluates once instead of per fragment.
    pub(crate) invariant: usize,
    /// The `evaluate` function for shape (ii), in [`Mode::Generate`].
    pub(crate) wgsl: Option<String>,
}

/// A place a value can come from: a named slot, or the zero an empty stack yields.
#[derive(Debug, Clone, Copy)]
enum Cell {
    Slot(usize),
    Zero,
}

/// What the walk knows about one live stack position.
#[derive(Debug, Clone, Copy)]
struct Info {
    kind: Kind,
    /// The literal it was pushed as, when it was pushed as one — `copy`, `index` and
    /// `roll` need their count at compile time in [`Mode::Generate`].
    literal: Option<f32>,
    /// Whether the value depends on the fragment's own coordinate.
    ///
    /// It is the whole asymmetry between the two shapes: a value that does not vary
    /// is one a shader compiler folds at compile time and an interpreter re-computes
    /// at every fragment. Both witnesses spend most of their instructions on
    /// arithmetic that does not vary — the BBP series is literals all the way down.
    varies: bool,
}

impl Info {
    const UNKNOWN: Self = Self {
        kind: Kind::Unknown,
        literal: None,
        varies: true,
    };

    fn of(kind: Kind, varies: bool) -> Self {
        Self {
            kind,
            literal: None,
            varies,
        }
    }

    fn merge(self, other: Self) -> Self {
        Self {
            kind: self.kind.merge(other.kind),
            literal: match (self.literal, other.literal) {
                (Some(a), Some(b)) if a.to_bits() == b.to_bits() => Some(a),
                _ => None,
            },
            varies: self.varies || other.varies,
        }
    }
}

/// Walk `program`, refusing anything the chosen [`Mode`] cannot express.
///
/// `limit` is the number of operand-stack slots available — the interpreter's array
/// size, and the generated shader's `var` count.
///
/// # Errors
///
/// A [`Refusal`] naming what the program does that the shape cannot.
pub(crate) fn walk(program: &Program, limit: usize, mode: Mode) -> Result<Facts, Refusal> {
    let mut walker = Walker {
        source: &program.ops,
        ops: program.ops.clone(),
        stack: Vec::new(),
        max: 0,
        limit,
        underflows: 0,
        branches: 0,
        invariant: 0,
        body: String::new(),
        mode,
    };
    for _ in 0..program.inputs {
        walker.push(Info::of(Kind::Real, true))?;
    }
    // The inputs arrive in the slots the walk just reserved.
    walker.line("s0 = x;");
    walker.line("s1 = y;");
    walker.region(0, program.ops.len())?;

    let wgsl = (mode == Mode::Generate).then(|| walker.finish(program.outputs));
    Ok(Facts {
        ops: walker.ops,
        max_depth: walker.max,
        underflows: walker.underflows,
        branches: walker.branches,
        invariant: walker.invariant,
        wgsl,
    })
}

struct Walker<'a> {
    source: &'a [Op],
    ops: Vec<Op>,
    stack: Vec<Info>,
    max: usize,
    limit: usize,
    underflows: usize,
    branches: usize,
    invariant: usize,
    body: String,
    mode: Mode,
}

impl Walker<'_> {
    fn generating(&self) -> bool {
        self.mode == Mode::Generate
    }

    fn line(&mut self, text: &str) {
        if self.generating() {
            self.body.push_str(text);
            self.body.push('\n');
        }
    }

    /// Record whether an instruction's result is one a compiler can fold away.
    fn note(&mut self, varies: bool) {
        if !varies {
            self.invariant = self.invariant.saturating_add(1);
        }
    }

    fn pop(&mut self) -> (Cell, Info) {
        if let Some(info) = self.stack.pop() {
            return (Cell::Slot(self.stack.len()), info);
        }
        self.underflows = self.underflows.saturating_add(1);
        (Cell::Zero, Info::UNKNOWN)
    }

    fn push(&mut self, info: Info) -> Result<usize, Refusal> {
        if self.stack.len() == self.limit {
            return Err(Refusal::StackTooDeep {
                needed: self.limit.saturating_add(1),
                limit: self.limit,
            });
        }
        let slot = self.stack.len();
        self.stack.push(info);
        self.max = self.max.max(self.stack.len());
        Ok(slot)
    }

    /// The expression that reads a cell.
    fn read(cell: Cell) -> String {
        match cell {
            Cell::Slot(index) => format!("s{index}"),
            Cell::Zero => "0.0".to_string(),
        }
    }

    /// The count operand of `copy`, `index` or `roll`, which shape (ii) needs
    /// statically. Shape (i) reads it from the stack and does not care.
    ///
    /// Shape (ii) cannot name a slot it cannot compute. Shape (i) could read the
    /// count from its stack — but the *walk* could not then say how deep the program
    /// goes, and a depth nobody knows is a refusal either way. No program in the
    /// caller's corpus reaches this.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the literal was pushed by `PushInt` and round-trips exactly"
    )]
    fn count(info: Info, name: &'static str) -> Result<i32, Refusal> {
        info.literal
            .map(|value| value as i32)
            .ok_or(Refusal::DynamicStackOperand(name))
    }

    /// Execute the instructions in `[start, end)`, which is one structured region.
    fn region(&mut self, start: usize, end: usize) -> Result<(), Refusal> {
        let mut pc = start;
        while pc < end {
            match self.source[pc] {
                Op::JumpIfFalse(target) => pc = self.conditional(pc, target as usize)?,
                // A `Jump` reaching the linear walk is the end of a then-arm, which
                // `conditional` has already accounted for; skipping it is correct.
                Op::Jump(target) => pc = target as usize,
                op => {
                    self.step(pc, op)?;
                    pc = pc.saturating_add(1);
                }
            }
        }
        Ok(())
    }

    /// One `if` or `ifelse`; returns the address after it.
    fn conditional(&mut self, pc: usize, target: usize) -> Result<usize, Refusal> {
        self.branches = self.branches.saturating_add(1);
        let (cell, test) = self.pop();
        let condition = Self::read(cell);

        // An `ifelse`'s then-arm ends with the `Jump` the emitter put there.
        let paired = target
            .checked_sub(1)
            .filter(|last| *last > pc)
            .and_then(|last| match self.source.get(last) {
                Some(Op::Jump(end)) if *end as usize >= target => Some((last, *end as usize)),
                _ => None,
            });

        self.line(&format!("if ({condition} != 0.0) {{"));
        let entry = self.stack.clone();
        let then_end = paired.map_or(target, |(last, _)| last);
        self.region(pc.saturating_add(1), then_end)?;
        let after_then = std::mem::replace(&mut self.stack, entry);

        let after = if let Some((_, end)) = paired {
            self.line("} else {");
            self.region(target, end)?;
            self.line("}");
            end
        } else {
            self.line("}");
            target
        };

        if after_then.len() != self.stack.len() {
            return Err(Refusal::UnbalancedBranches {
                then_depth: after_then.len(),
                else_depth: self.stack.len(),
            });
        }
        for (slot, info) in self.stack.iter_mut().enumerate() {
            *info = info.merge(after_then[slot]);
            info.varies |= test.varies;
        }
        Ok(after)
    }

    fn step(&mut self, pc: usize, op: Op) -> Result<(), Refusal> {
        match op {
            Op::PushReal(_) | Op::PushInt(_) | Op::PushBool(_) => self.step_push(op),
            Op::Copy | Op::Dup | Op::Exch | Op::Index | Op::Pop | Op::Roll => self.step_stack(op),
            Op::Not => self.step_not(pc),
            _ if arity(op) == 1 => self.step_unary(op),
            _ => self.step_binary(op),
        }
    }

    fn step_push(&mut self, op: Op) -> Result<(), Refusal> {
        let (value, info) = match op {
            Op::PushReal(value) => (
                value,
                Info {
                    kind: Kind::Real,
                    literal: Some(value),
                    varies: false,
                },
            ),
            #[allow(
                clippy::cast_precision_loss,
                reason = "a literal beyond 2^24 is already inexact in the caller's f32 form"
            )]
            Op::PushInt(value) => (
                value as f32,
                Info {
                    kind: Kind::Int,
                    literal: Some(value as f32),
                    varies: false,
                },
            ),
            _ => {
                let value = f32::from(op == Op::PushBool(true));
                (
                    value,
                    Info {
                        kind: Kind::Bool,
                        literal: Some(value),
                        varies: false,
                    },
                )
            }
        };
        self.invariant = self.invariant.saturating_add(1);
        let slot = self.push(info)?;
        self.line(&format!("s{slot} = {value:?};"));
        Ok(())
    }

    fn step_unary(&mut self, op: Op) -> Result<(), Refusal> {
        let (cell, info) = self.pop();
        let operand = Self::read(cell);
        self.note(info.varies);
        let slot = self.push(Info::of(unary_kind(op, info.kind), info.varies))?;
        self.line(&format!(
            "{{ let t = {operand}; s{slot} = {}(t); }}",
            wgsl_name(op)
        ));
        Ok(())
    }

    fn step_binary(&mut self, op: Op) -> Result<(), Refusal> {
        let (right_cell, right) = self.pop();
        let (left_cell, left) = self.pop();
        let (a, b) = (Self::read(left_cell), Self::read(right_cell));
        let varies = left.varies || right.varies;
        self.note(varies);
        let slot = self.push(Info::of(binary_kind(op, left.kind, right.kind), varies))?;
        let call = wgsl_name(op);
        self.line(&format!(
            "{{ let a = {a}; let b = {b}; s{slot} = {call}(a, b); }}"
        ));
        Ok(())
    }

    /// Table 42's `not` is two operators wearing one name; this is where they part.
    fn step_not(&mut self, pc: usize) -> Result<(), Refusal> {
        let (cell, info) = self.pop();
        let resolved = match info.kind {
            Kind::Bool => Op::Not,
            Kind::Int => Op::BitNot,
            Kind::Real | Kind::Unknown => return Err(Refusal::AmbiguousKind("not")),
        };
        self.ops[pc] = resolved;
        let operand = Self::read(cell);
        self.note(info.varies);
        let slot = self.push(Info::of(info.kind, info.varies))?;
        self.line(&format!(
            "{{ let t = {operand}; s{slot} = {}(t); }}",
            wgsl_name(resolved)
        ));
        Ok(())
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the six stack operators of Table 42 are one table; splitting it hides the shape"
    )]
    fn step_stack(&mut self, op: Op) -> Result<(), Refusal> {
        match op {
            Op::Pop => {
                self.pop();
            }
            Op::Dup => {
                let (cell, info) = self.pop();
                let operand = Self::read(cell);
                let first = self.push(info)?;
                let second = self.push(info)?;
                self.line(&format!(
                    "{{ let t = {operand}; s{first} = t; s{second} = t; }}"
                ));
            }
            Op::Exch => {
                let (right_cell, right) = self.pop();
                let (left_cell, left) = self.pop();
                let (a, b) = (Self::read(left_cell), Self::read(right_cell));
                let first = self.push(right)?;
                let second = self.push(left)?;
                self.line(&format!(
                    "{{ let a = {a}; let b = {b}; s{first} = b; s{second} = a; }}"
                ));
            }
            Op::Index => {
                let (_, count) = self.pop();
                let n = Self::count(count, "index")?;
                let depth = self.stack.len();
                let source = usize::try_from(n)
                    .ok()
                    .and_then(|n| depth.checked_sub(n.saturating_add(1)));
                let (cell, info) = if let Some(index) = source {
                    (Cell::Slot(index), self.stack[index])
                } else {
                    self.underflows = self.underflows.saturating_add(1);
                    (Cell::Zero, Info::UNKNOWN)
                };
                let operand = Self::read(cell);
                let slot = self.push(info)?;
                self.line(&format!("s{slot} = {operand};"));
            }
            Op::Copy => {
                let (_, count) = self.pop();
                let n = usize::try_from(Self::count(count, "copy")?).unwrap_or(0);
                let depth = self.stack.len();
                if n > depth {
                    self.underflows = self.underflows.saturating_add(1);
                    return Ok(());
                }
                let base = depth.saturating_sub(n);
                let sources: Vec<Info> = self.stack[base..depth].to_vec();
                for (offset, info) in sources.into_iter().enumerate() {
                    let slot = self.push(info)?;
                    self.line(&format!("s{slot} = s{};", base.saturating_add(offset)));
                }
            }
            Op::Roll => {
                let (_, by) = self.pop();
                let j = Self::count(by, "roll")?;
                let (_, count) = self.pop();
                let n = Self::count(count, "roll")?;
                self.roll(n, j);
            }
            _ => unreachable!("step_stack is only called for the six stack operators"),
        }
        Ok(())
    }

    /// `n j roll`: the top `n` slots rotated toward the top by `j`.
    fn roll(&mut self, count: i32, by: i32) {
        let depth = self.stack.len();
        let Ok(n) = usize::try_from(count) else {
            return;
        };
        if n == 0 || n > depth {
            self.underflows = self.underflows.saturating_add(1);
            return;
        }
        let shift = by.rem_euclid(count);
        let Ok(shift) = usize::try_from(shift) else {
            return;
        };
        let base = depth.saturating_sub(n);
        self.stack[base..depth].rotate_right(shift);
        if self.generating() && shift != 0 {
            let temporaries: Vec<String> = (0..n)
                .map(|k| format!("let t{k} = s{};", base.saturating_add(k)))
                .collect();
            let writes: Vec<String> = (0..n)
                .map(|k| {
                    let from = (k + n - shift) % n;
                    format!("s{} = t{from};", base.saturating_add(k))
                })
                .collect();
            self.line(&format!(
                "{{ {} {} }}",
                temporaries.join(" "),
                writes.join(" ")
            ));
        }
    }

    /// Wrap the body in `evaluate`, with one `var` per operand-stack slot.
    fn finish(&self, outputs: usize) -> String {
        let mut source = String::from("fn evaluate(x: f32, y: f32) -> vec3<f32> {\n");
        for slot in 0..self.max {
            let _ = writeln!(source, "var s{slot}: f32 = 0.0;");
        }
        // The body is already a sequence of complete lines.
        source.push_str(&self.body);
        let depth = self.stack.len();
        let component = |back: usize| -> String {
            depth
                .checked_sub(back)
                .map_or_else(|| "0.0".to_string(), |index| format!("s{index}"))
        };
        let _ = writeln!(
            source,
            "return vec3<f32>({}, {}, {});\n}}",
            component(outputs),
            component(outputs.saturating_sub(1)),
            component(outputs.saturating_sub(2))
        );
        source
    }
}

/// How many operands an arithmetic or relational operator takes.
fn arity(op: Op) -> usize {
    match op {
        Op::Abs
        | Op::Ceiling
        | Op::Cos
        | Op::Cvi
        | Op::Cvr
        | Op::Floor
        | Op::Ln
        | Op::Log
        | Op::Neg
        | Op::Round
        | Op::Sin
        | Op::Sqrt
        | Op::Truncate
        | Op::Not
        | Op::BitNot => 1,
        _ => 2,
    }
}

/// The kind a one-operand operator leaves, given the kind it consumed.
fn unary_kind(op: Op, operand: Kind) -> Kind {
    match op {
        Op::Abs | Op::Neg | Op::Ceiling | Op::Floor | Op::Round | Op::Truncate => operand,
        Op::Cvi | Op::BitNot => Kind::Int,
        Op::Not => Kind::Bool,
        _ => Kind::Real,
    }
}

/// The kind a two-operand operator leaves.
fn binary_kind(op: Op, left: Kind, right: Kind) -> Kind {
    match op {
        // §7.10.5.2: these are integer-preserving when both operands are integers.
        Op::Add | Op::Sub | Op::Mul => {
            if left == Kind::Int && right == Kind::Int {
                Kind::Int
            } else {
                Kind::Real
            }
        }
        Op::IDiv | Op::Mod | Op::BitShift => Kind::Int,
        Op::And | Op::Or | Op::Xor => {
            if left == Kind::Bool && right == Kind::Bool {
                Kind::Bool
            } else {
                Kind::Int
            }
        }
        Op::Eq | Op::Ne | Op::Ge | Op::Gt | Op::Le | Op::Lt => Kind::Bool,
        _ => Kind::Real,
    }
}

/// The `ops.wgsl` function that implements an operator.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per Table 42 operator, which is the table this maps"
)]
fn wgsl_name(op: Op) -> &'static str {
    match op {
        Op::Abs => "ps_abs",
        Op::Add => "ps_add",
        Op::Atan => "ps_atan",
        Op::Ceiling => "ps_ceiling",
        Op::Cos => "ps_cos",
        Op::Cvi => "ps_cvi",
        Op::Div => "ps_div",
        Op::Exp => "ps_exp",
        Op::Floor => "ps_floor",
        Op::IDiv => "ps_idiv",
        Op::Ln => "ps_ln",
        Op::Log => "ps_log",
        Op::Mod => "ps_mod",
        Op::Mul => "ps_mul",
        Op::Neg => "ps_neg",
        Op::Round => "ps_round",
        Op::Sin => "ps_sin",
        Op::Sqrt => "ps_sqrt",
        Op::Sub => "ps_sub",
        Op::Truncate => "ps_truncate",
        Op::And => "ps_and",
        Op::BitShift => "ps_bitshift",
        Op::Eq => "ps_eq",
        Op::Ge => "ps_ge",
        Op::Gt => "ps_gt",
        Op::Le => "ps_le",
        Op::Lt => "ps_lt",
        Op::Ne => "ps_ne",
        Op::Not => "ps_not",
        Op::BitNot => "ps_bit_not",
        Op::Or => "ps_or",
        Op::Xor => "ps_xor",
        // `cvr` is the identity, and the pushes, the stack operators and the jumps
        // have no `ps_*` of their own: `walk` emits their assignments directly and
        // never asks for a name. They share the identity for the same reason.
        Op::Cvr
        | Op::PushReal(_)
        | Op::PushInt(_)
        | Op::PushBool(_)
        | Op::Copy
        | Op::Dup
        | Op::Exch
        | Op::Index
        | Op::Pop
        | Op::Roll
        | Op::JumpIfFalse(_)
        | Op::Jump(_) => "ps_cvr",
    }
}
