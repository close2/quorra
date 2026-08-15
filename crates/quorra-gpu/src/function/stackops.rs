//! ISO 32000-2 Table 42's six stack operators, which are the ones with a count.
//!
//! One responsibility: **`copy`, `dup`, `exch`, `index`, `pop` and `roll` over the walk's
//! compile-time operand stack.** They are here rather than in [`super::analyse`] because
//! they are the only operators whose *operands decide which slots the shader touches*, and
//! that is the whole of the pinned decision "`copy`, `index` and `roll` must have a
//! statically resolvable count, or the program is refused".
//!
//! All six lower to [`Step::Permute`] and to nothing else. That is not a simplification: a
//! stack operator moves values, and the reads-before-writes invariant `Permute` carries is
//! exactly what a slot-indexed model needs to express `exch` without losing one of the two
//! values.
//!
//! # The count, and why "not a literal" is a refusal rather than a fallback
//!
//! Shape (i) of the spike — an interpreter with a run-time operand stack — could read a
//! computed count and would not care. It is not the shape being built
//! (`doc/adr/0053` §1: 133 ms against 0.060, and it lost the device at 4×). A generated
//! shader cannot name a slot it cannot compute, and a walk that cannot resolve the count
//! cannot state the program's own depth either, so *both* halves of the admission fail on
//! the same instruction. Neither of the caller's two witnesses reaches this.

use quorra_scene::FnOp;

use super::analyse::slot_index;
use super::lowered::{Source, Step};
use super::refusal::FunctionRefusal;
use super::walk::{Cell, Walk};

impl Walk<'_> {
    /// One of the six, over the compile-time operand stack.
    pub(super) fn stack_operator(
        &mut self,
        op: FnOp,
        out: &mut Vec<Step>,
    ) -> Result<(), FunctionRefusal> {
        match op {
            FnOp::Pop => {
                self.pop();
                Ok(())
            }
            FnOp::Dup => self.dup(out),
            FnOp::Exch => self.exch(out),
            FnOp::Index => self.index(out),
            FnOp::Copy => self.copy(out),
            FnOp::Roll => self.roll(out),
            // `stack_operator` is called for exactly the six; anything else here would be a
            // dispatch that lost track of its own table, and a wrong slot is a wrong colour.
            _ => Err(FunctionRefusal::DynamicStackCount {
                operator: "a stack operator",
            }),
        }
    }

    /// `dup`: the top operand, twice.
    ///
    /// A pop and two pushes rather than one push, deliberately: on an empty stack this is
    /// [`Source::EmptyStackZero`] duplicated, which leaves the stack one deeper than it
    /// started — the same thing PostScript's own `dup` would do if it had a value to
    /// duplicate.
    fn dup(&mut self, out: &mut Vec<Step>) -> Result<(), FunctionRefusal> {
        let (source, cell) = self.pop();
        let first = self.push(cell)?;
        let second = self.push(cell)?;
        emit(out, vec![(first, source), (second, source)]);
        Ok(())
    }

    /// `exch`: the top two operands, exchanged.
    fn exch(&mut self, out: &mut Vec<Step>) -> Result<(), FunctionRefusal> {
        let (upper_source, upper) = self.pop();
        let (lower_source, lower) = self.pop();
        let first = self.push(upper)?;
        let second = self.push(lower)?;
        emit(out, vec![(first, upper_source), (second, lower_source)]);
        Ok(())
    }

    /// `n index`: a copy of the operand *n* below the top, after the count itself is popped.
    fn index(&mut self, out: &mut Vec<Step>) -> Result<(), FunctionRefusal> {
        let count = self.integer_operand("index")?;
        let depth = self.stack.len();
        let out_of_range = || FunctionRefusal::StackCountOutOfRange {
            operator: "index",
            count,
            depth,
        };
        let below = usize::try_from(count).map_err(|_| out_of_range())?;
        // `0 index` is the top, so the deepest reachable operand is `depth - 1`.
        let from = depth
            .checked_sub(1)
            .and_then(|top| top.checked_sub(below))
            .ok_or_else(out_of_range)?;
        let cell = *self.stack.get(from).ok_or_else(out_of_range)?;
        let slot = self.push(cell)?;
        emit(out, vec![(slot, Source::Slot(slot_index(from)))]);
        Ok(())
    }

    /// `n copy`: the top *n* operands, duplicated in order.
    fn copy(&mut self, out: &mut Vec<Step>) -> Result<(), FunctionRefusal> {
        let count = self.integer_operand("copy")?;
        let depth = self.stack.len();
        let out_of_range = || FunctionRefusal::StackCountOutOfRange {
            operator: "copy",
            count,
            depth,
        };
        let wanted = usize::try_from(count).map_err(|_| out_of_range())?;
        let base = depth.checked_sub(wanted).ok_or_else(out_of_range)?;
        let sources: Vec<Cell> = self
            .stack
            .get(base..depth)
            .ok_or_else(out_of_range)?
            .to_vec();
        let mut writes = Vec::with_capacity(wanted);
        for (offset, cell) in sources.into_iter().enumerate() {
            let slot = self.push(cell)?;
            writes.push((slot, Source::Slot(slot_index(base.saturating_add(offset)))));
        }
        emit(out, writes);
        Ok(())
    }

    /// `n j roll`: the top *n* operands rotated by *j*, positive *j* moving them up.
    fn roll(&mut self, out: &mut Vec<Step>) -> Result<(), FunctionRefusal> {
        // `j` is on top, so it comes off first.
        let by = self.integer_operand("roll")?;
        let count = self.integer_operand("roll")?;
        let depth = self.stack.len();
        let out_of_range = || FunctionRefusal::StackCountOutOfRange {
            operator: "roll",
            count,
            depth,
        };
        let wanted = usize::try_from(count).map_err(|_| out_of_range())?;
        let base = depth.checked_sub(wanted).ok_or_else(out_of_range)?;
        if wanted == 0 {
            // PLRM3 leaves the stack alone; `j` is not consulted, so a `j` of any size is
            // legal here and refusing one would refuse a legal program.
            return Ok(());
        }
        // `count` is positive, so the remainder is in `0..count` and the conversion is total.
        let shift = usize::try_from(by.rem_euclid(count)).map_err(|_| out_of_range())?;
        if shift == 0 {
            return Ok(());
        }

        // The permutation, built by rotating the *positions* rather than by arithmetic on
        // them: `order[k]` is the offset whose value ends up at offset `k`.
        let mut order: Vec<usize> = (0..wanted).collect();
        order.rotate_right(shift);
        let writes = order
            .iter()
            .enumerate()
            .map(|(target, from)| {
                (
                    slot_index(base.saturating_add(target)),
                    Source::Slot(slot_index(base.saturating_add(*from))),
                )
            })
            .collect();

        let Some(window) = self.stack.get_mut(base..depth) else {
            return Err(out_of_range());
        };
        window.rotate_right(shift);
        emit(out, writes);
        Ok(())
    }

    /// The integer operand of `copy`, `index` or `roll`.
    ///
    /// Refuses when it is not a value the walk can name, and when it is not an integer:
    /// PLRM3 requires an integer of all three, and lowering `2.7 index` by truncation would
    /// name a slot the program never asked for.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::float_cmp,
        reason = "Rust's float-to-integer `as` saturates, which is the clamp a value this far \
                  outside any admissible count wants; and `trunc() != value` is exactly the \
                  `typecheck` PLRM3 states for a non-integer count — an epsilon there would \
                  admit `2.0000001 index` and name a slot the program never asked for"
    )]
    fn integer_operand(&mut self, operator: &'static str) -> Result<i64, FunctionRefusal> {
        let (_, cell) = self.pop();
        let Some(value) = cell.literal else {
            return Err(FunctionRefusal::DynamicStackCount { operator });
        };
        if !value.is_finite() || value.trunc() != value {
            return Err(FunctionRefusal::OperandType {
                operator,
                required: "an integer",
                found: "a real",
            });
        }
        Ok(value as i64)
    }
}

/// Emit a permutation, less the writes that would move a value to where it already is.
///
/// `0 index`, `n 0 roll` and the lower half of a `dup` are all identities in the slot model,
/// and an identity assignment in the WGSL is a line a reader has to prove is a no-op.
fn emit(out: &mut Vec<Step>, writes: Vec<(u32, Source)>) {
    let writes: Vec<(u32, Source)> = writes
        .into_iter()
        .filter(|(slot, source)| *source != Source::Slot(*slot))
        .collect();
    if !writes.is_empty() {
        out.push(Step::Permute { writes });
    }
}
