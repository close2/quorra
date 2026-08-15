//! The compile-time operand stack, and the one recursive descent over the instruction list.
//!
//! One responsibility: **execute a §7.10.5 program symbolically.** Every fact
//! [`super::Analysis`] reports is produced here, on one pass, by running the program over a
//! stack of *what the walk knows about a value* rather than of values: its type, the literal
//! it was pushed as where it was pushed as one, and whether an inexact operator is upstream
//! of it.
//!
//! Three modules meet here, and they answer three questions. [`super::analyse`] owns *what an
//! admitted program is* — the type a caller holds, the budgets, the `Range` a shading brings
//! later. [`super::typing`] owns *what type each operator leaves and where a wrong one is a
//! refusal*. This one owns *how the instruction list is read*: the stack, the branches, the
//! taint, and the [`Step`]s that come out.
//!
//! # Why a walk is enough
//!
//! Because the jumps are forward-only *and nest*. [`super::validate_jumps`] proves the first
//! before this module runs, and [`Walk::conditional`] proves the second by construction: an
//! unconditional jump reached in sequence is a `goto`, no §7.10.5 source can produce one
//! (`if` and `ifelse` are the only control flow §7.10.5.1 admits), and a generated shader has
//! nothing to lower it to. So the list reads as a tree.

use quorra_scene::FnOp;

use super::MAX_BRANCH_NESTING;
use super::MAX_OPERAND_SLOTS;
use super::analyse::{INPUTS, slot_index};
use super::lowered::{Agreement, SlotType, Source, Step};
use super::operators::{Binary, Unary};
use super::refusal::FunctionRefusal;
use super::typing::{binary_of, binary_result, check_operand_types, unary_of, unary_result};

/// The provenance of a value that an inexact operator computed, kept so that a refusal or a
/// classification can name the operator rather than the symptom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Taint {
    pub(super) operator: &'static str,
    pub(super) at: usize,
}

/// Whichever taint came first in the program, which makes the classification a function of
/// the program rather than of the order the walk happened to visit operands in.
pub(super) fn earliest(left: Option<Taint>, right: Option<Taint>) -> Option<Taint> {
    match (left, right) {
        (Some(a), Some(b)) => Some(if a.at <= b.at { a } else { b }),
        (only @ Some(_), None) | (None, only) => only,
    }
}

/// What the walk knows about one live operand-stack position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Cell {
    pub(super) ty: SlotType,
    /// The value, where it is one the walk can name. `copy`, `index` and `roll` need their
    /// count at compile time, and this is the only thing that supplies it.
    pub(super) literal: Option<f32>,
    pub(super) taint: Option<Taint>,
}

impl Cell {
    /// One of the two coordinates §8.7.4.5.2 hands the function.
    pub(super) const INPUT: Self = Self {
        ty: SlotType::Real,
        literal: None,
        taint: None,
    };

    /// The zero a pop from an empty operand stack yields.
    ///
    /// **Integer `0`, and the type is as deliberate as the value.** ISO 32000-2 defines
    /// neither, and PostScript would raise `stackunderflow`; seven of Table 42's operators
    /// (`not`, `and`, `or`, `xor`, `idiv`, `mod`, `bitshift`) can tell an integer from a
    /// real, so leaving the type open would turn a decision about the value into a refusal
    /// about the type. `Analysis::empty_stack_pops` counts these so the decision is reported
    /// rather than adopted invisibly.
    pub(super) const EMPTY_STACK: Self = Self {
        ty: SlotType::Integer,
        literal: Some(0.0),
        taint: None,
    };

    pub(super) fn of(ty: SlotType, taint: Option<Taint>) -> Self {
        Self {
            ty,
            literal: None,
            taint,
        }
    }

    /// The cell a slot holds where two arms of an `ifelse` wrote different ones.
    fn merge(self, other: Self) -> Self {
        Self {
            ty: self.ty.merge(other.ty),
            literal: match (self.literal, other.literal) {
                // Bit patterns, not `==`: two arms that both leave `0.0` and `-0.0` have
                // not left the same value, and a count is read from this.
                (Some(a), Some(b)) if a.to_bits() == b.to_bits() => Some(a),
                _ => None,
            },
            taint: earliest(self.taint, other.taint),
        }
    }
}

/// The compile-time operand stack, and the facts it accumulates.
pub(super) struct Walk<'a> {
    pub(super) program: &'a [FnOp],
    pub(super) stack: Vec<Cell>,
    pub(super) max_depth: usize,
    pub(super) empty_stack_pops: u32,
    /// The *first* amplification in program order, which is the one a reader should be told
    /// about; later ones say nothing new.
    pub(super) approximate: Option<Agreement>,
}

impl<'a> Walk<'a> {
    pub(super) fn new(program: &'a [FnOp]) -> Self {
        Self {
            program,
            stack: vec![Cell::INPUT; INPUTS],
            max_depth: INPUTS,
            empty_stack_pops: 0,
            approximate: None,
        }
    }

    /// Pop one operand, yielding [`Source::EmptyStackZero`] when there is none.
    pub(super) fn pop(&mut self) -> (Source, Cell) {
        if let Some(cell) = self.stack.pop() {
            return (Source::Slot(slot_index(self.stack.len())), cell);
        }
        self.empty_stack_pops = self.empty_stack_pops.saturating_add(1);
        (Source::EmptyStackZero, Cell::EMPTY_STACK)
    }

    /// Push one operand, refusing at the stated budget.
    pub(super) fn push(&mut self, cell: Cell) -> Result<u32, FunctionRefusal> {
        let slot = self.stack.len();
        if slot >= MAX_OPERAND_SLOTS {
            return Err(FunctionRefusal::StackTooDeep {
                needed: slot.saturating_add(1),
                limit: MAX_OPERAND_SLOTS,
            });
        }
        self.stack.push(cell);
        self.max_depth = self.max_depth.max(self.stack.len());
        Ok(slot_index(slot))
    }

    /// Record an amplification, if this operator is one and its operand carries a taint.
    pub(super) fn amplify(
        &mut self,
        at: usize,
        taint: Option<Taint>,
        is_amplifier: bool,
        amplifier: &'static str,
    ) {
        if !is_amplifier || self.approximate.is_some() {
            return;
        }
        if let Some(taint) = taint {
            self.approximate = Some(Agreement::Unbounded {
                inexact: taint.operator,
                inexact_at: taint.at,
                amplifier,
                amplifier_at: at,
            });
        }
    }

    /// The taint a result carries: its operands', or this operator's own where it is the
    /// first inexact one in the chain.
    pub(super) fn taint_after(
        at: usize,
        operands: Option<Taint>,
        inexact: bool,
        operator: &'static str,
    ) -> Option<Taint> {
        match (operands, inexact) {
            (Some(taint), _) => Some(taint),
            (None, true) => Some(Taint { operator, at }),
            (None, false) => None,
        }
    }

    /// Execute the instructions in `[start, end)`, which is one structured region.
    pub(super) fn region(
        &mut self,
        start: usize,
        end: usize,
        nesting: usize,
        out: &mut Vec<Step>,
    ) -> Result<(), FunctionRefusal> {
        let mut pc = start;
        while pc < end {
            let Some(op) = self.program.get(pc) else {
                return Err(FunctionRefusal::UnstructuredControlFlow { at: pc });
            };
            match *op {
                FnOp::JumpUnless { target } => {
                    let target =
                        usize::try_from(target).map_err(|_| FunctionRefusal::JumpOutOfRange {
                            at: pc,
                            target,
                            length: self.program.len(),
                        })?;
                    pc = self.conditional(pc, target, end, nesting, out)?;
                }
                // Reached in sequence rather than as an `ifelse`'s arm terminator, this is a
                // `goto`: nothing `if`/`ifelse` compiles to, and nothing a generated shader
                // can express.
                FnOp::Jump { .. } => {
                    return Err(FunctionRefusal::UnstructuredControlFlow { at: pc });
                }
                op => {
                    self.step(pc, op, out)?;
                    pc = pc.saturating_add(1);
                }
            }
        }
        Ok(())
    }

    /// One `if` or `ifelse`; returns the address after it.
    fn conditional(
        &mut self,
        pc: usize,
        target: usize,
        end: usize,
        nesting: usize,
        out: &mut Vec<Step>,
    ) -> Result<usize, FunctionRefusal> {
        if nesting >= MAX_BRANCH_NESTING {
            return Err(FunctionRefusal::BranchesTooDeep {
                at: pc,
                limit: MAX_BRANCH_NESTING,
            });
        }
        // A conditional that jumps out of the region containing it does not nest.
        if target > end {
            return Err(FunctionRefusal::UnstructuredControlFlow { at: pc });
        }
        let (condition, cell) = self.pop();
        // §7.10.5.2's `if`/`ifelse` take a boolean, and PostScript makes anything else a
        // `typecheck`. Lowering a real as `!= 0.0` would branch on a value the standard never
        // called a condition, so it is refused instead.
        match cell.ty {
            SlotType::Boolean => {}
            SlotType::Undecided => {
                return Err(FunctionRefusal::UndecidableOperandType { operator: "if" });
            }
            other => {
                return Err(FunctionRefusal::OperandType {
                    operator: "if",
                    required: "a boolean",
                    found: other.name(),
                });
            }
        }
        // Kept although the refusal above makes it unreachable: it is what states that a
        // branch amplifies, independently of who else happens to be checking types. The
        // classification stays correct if that check is ever relaxed, and a classification
        // that depended on a refusal for its correctness would be two rules pretending to be
        // one.
        self.amplify(pc, cell.taint, true, "if");

        let paired = self.paired_jump(pc, target, end);
        let inner = nesting.saturating_add(1);
        let entry = self.stack.clone();

        let mut on_true = Vec::new();
        self.region(
            pc.saturating_add(1),
            paired.map_or(target, |(last, _)| last),
            inner,
            &mut on_true,
        )?;
        let after_true = std::mem::replace(&mut self.stack, entry);

        let mut on_false = Vec::new();
        let after = match paired {
            Some((_, resume)) => {
                self.region(target, resume, inner, &mut on_false)?;
                resume
            }
            None => target,
        };

        if after_true.len() != self.stack.len() {
            return Err(FunctionRefusal::UnbalancedBranches {
                at: pc,
                then_depth: after_true.len(),
                else_depth: self.stack.len(),
            });
        }
        for (slot, cell) in self.stack.iter_mut().enumerate() {
            if let Some(other) = after_true.get(slot) {
                *cell = cell.merge(*other);
            }
        }
        out.push(Step::Branch {
            condition,
            on_true,
            on_false,
        });
        Ok(after)
    }

    /// The `Jump` that ends an `ifelse`'s first arm, where there is one: its address and the
    /// address the whole conditional resumes at.
    ///
    /// An `if` compiles to a `JumpUnless` alone, an `ifelse` to a `JumpUnless` whose first
    /// arm ends in a `Jump` over the second. Distinguishing them is exactly this lookup.
    fn paired_jump(&self, pc: usize, target: usize, end: usize) -> Option<(usize, usize)> {
        let last = target.checked_sub(1).filter(|last| *last > pc)?;
        let FnOp::Jump { target: resume } = *self.program.get(last)? else {
            return None;
        };
        let resume = usize::try_from(resume).ok()?;
        (resume >= target && resume <= end).then_some((last, resume))
    }

    /// One instruction that is not control flow.
    fn step(&mut self, pc: usize, op: FnOp, out: &mut Vec<Step>) -> Result<(), FunctionRefusal> {
        match op {
            FnOp::PushReal(value) => self.literal(pc, value, SlotType::Real, out),
            #[allow(
                clippy::cast_precision_loss,
                reason = "the operand stack is f32; an integer literal past 2^24 is already \
                          inexact in the compiled form the caller hands us"
            )]
            FnOp::PushInt(value) => self.literal(pc, value as f32, SlotType::Integer, out),
            FnOp::PushBool(value) => self.literal(pc, f32::from(value), SlotType::Boolean, out),
            FnOp::Not => self.not(pc, out),
            FnOp::Copy | FnOp::Dup | FnOp::Exch | FnOp::Index | FnOp::Pop | FnOp::Roll => {
                self.stack_operator(op, out)
            }
            _ => match (unary_of(op), binary_of(op)) {
                (Some(unary), _) => self.unary(pc, unary, out),
                (None, Some(binary)) => self.binary(pc, binary, out),
                // Only `Jump` and `JumpUnless` are left, and `region` handles both before
                // calling here. Reaching this is a control-flow list that does not nest.
                (None, None) => Err(FunctionRefusal::UnstructuredControlFlow { at: pc }),
            },
        }
    }

    fn literal(
        &mut self,
        pc: usize,
        value: f32,
        ty: SlotType,
        out: &mut Vec<Step>,
    ) -> Result<(), FunctionRefusal> {
        if !value.is_finite() {
            return Err(FunctionRefusal::NonFiniteLiteral { at: pc, value });
        }
        let slot = self.push(Cell {
            ty,
            literal: Some(value),
            taint: None,
        })?;
        out.push(Step::Literal { slot, value });
        Ok(())
    }

    fn unary(&mut self, pc: usize, op: Unary, out: &mut Vec<Step>) -> Result<(), FunctionRefusal> {
        let (operand, cell) = self.pop();
        self.amplify(pc, cell.taint, op.amplifies(), op.table_42());
        let taint = Self::taint_after(pc, cell.taint, op.is_inexact(), op.table_42());
        let slot = self.push(Cell::of(unary_result(op, cell.ty), taint))?;
        out.push(Step::Unary { slot, op, operand });
        Ok(())
    }

    /// Table 42's `not` is two operators wearing one name; this is where they part.
    fn not(&mut self, pc: usize, out: &mut Vec<Step>) -> Result<(), FunctionRefusal> {
        let (operand, cell) = self.pop();
        let op = match cell.ty {
            SlotType::Boolean => Unary::LogicalNot,
            SlotType::Integer => Unary::BitwiseNot,
            SlotType::Real => {
                return Err(FunctionRefusal::OperandType {
                    operator: "not",
                    required: "a boolean or an integer",
                    found: SlotType::Real.name(),
                });
            }
            SlotType::Undecided => {
                return Err(FunctionRefusal::UndecidableOperandType { operator: "not" });
            }
        };
        self.amplify(pc, cell.taint, op.amplifies(), "not");
        let taint = Self::taint_after(pc, cell.taint, op.is_inexact(), "not");
        let slot = self.push(Cell::of(unary_result(op, cell.ty), taint))?;
        out.push(Step::Unary { slot, op, operand });
        Ok(())
    }

    fn binary(
        &mut self,
        pc: usize,
        op: Binary,
        out: &mut Vec<Step>,
    ) -> Result<(), FunctionRefusal> {
        let (right, right_cell) = self.pop();
        let (left, left_cell) = self.pop();
        check_operand_types(op, left_cell.ty, right_cell.ty)?;
        let operands = earliest(left_cell.taint, right_cell.taint);
        self.amplify(pc, operands, op.amplifies(), op.table_42());
        let taint = Self::taint_after(pc, operands, op.is_inexact(), op.table_42());
        let slot = self.push(Cell::of(
            binary_result(op, left_cell.ty, right_cell.ty),
            taint,
        ))?;
        out.push(Step::Binary {
            slot,
            op,
            left,
            right,
        });
        Ok(())
    }
}
