//! The one walk that admits a §7.10.5 program, or refuses it by name.
//!
//! One responsibility: **turn a flat instruction list into a set of static facts, or into a
//! refusal.** Every question the generated shader needs answered is answered here and
//! nowhere else, on one pass with a compile-time operand stack:
//!
//! - **how deep the operand stack goes**, which is how many `var`s the shader declares —
//!   computed, never supplied, because a caller cannot lie about a number we derived;
//! - **the static type of every slot**, which is what lets Table 42's `not` be lowered to
//!   the right one of its two meanings without a run-time type tag;
//! - **the count of every `copy`, `index` and `roll`**, because a generated shader cannot
//!   name a slot it cannot compute;
//! - **whether an inexact operator reaches an amplifier** — `doc/adr/0053` §3's
//!   classification, and the reason a program can be accepted at all;
//! - **whether the program pops an empty operand stack**, which is a decision rather than a
//!   reading and must therefore be reported.
//!
//! It is pure: no device, no allocation from an unchecked number, and every budget it
//! enforces is a public constant a caller can compare against before the frame
//! ([`MAX_PROGRAM_LENGTH`](super::MAX_PROGRAM_LENGTH),
//! [`MAX_OPERAND_SLOTS`](super::MAX_OPERAND_SLOTS),
//! [`MAX_BRANCH_NESTING`](super::MAX_BRANCH_NESTING)).
//!
//! The walk itself is [`super::walk`]: this module owns *what an admitted program is* and
//! the boundary a caller meets, that one owns *how the instruction list is read*. The seam is
//! where Table 42 stops and this library'''s vocabulary starts.

use quorra_scene::{FnOp, FnRange};

use super::MAX_PROGRAM_LENGTH;
use super::hash::ProgramHash;
use super::lowered::{Agreement, SlotType, Step};
use super::refusal::FunctionRefusal;
use super::walk::Walk;

/// What ISO 32000-2 §8.7.4.5.2 hands a type 1 shading's function: the two coordinates of
/// the point being coloured, already clipped to the `Domain`.
pub const INPUTS: usize = 2;

/// Everything the generator, the pipeline cache and a `Report` need to know about an
/// admitted program.
///
/// Constructed only by [`analyse`], so its invariants hold by construction: the depth is
/// within [`MAX_OPERAND_SLOTS`](super::MAX_OPERAND_SLOTS), every `copy`/`index`/`roll` count
/// is resolved, every `not` is resolved to one of its two meanings, and every jump nests.
///
/// It knows nothing about a `Range`, a `Domain`, a `Matrix` or a `Background` — those belong
/// to the *shading*, and one uploaded program may serve several. [`Analysis::admits`] is
/// where a `Range` meets a program.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    steps: Vec<Step>,
    max_depth: u32,
    slot_types: Vec<SlotType>,
    values_left: u32,
    agreement: Agreement,
    empty_stack_pops: u32,
    program_length: usize,
    hash: ProgramHash,
}

impl Analysis {
    /// The lowered program, in execution order.
    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The greatest operand-stack depth the program reaches, the two inputs included.
    ///
    /// This is the number of `var`s the generated shader declares, and it is the reason the
    /// caller is never asked for it: a supplied depth is a claim, and a claim that sizes a
    /// shader's storage is a claim a wrong page rests on.
    #[must_use]
    pub fn max_depth(&self) -> u32 {
        self.max_depth
    }

    /// The static type of every operand-stack slot the program leaves behind, bottom first.
    #[must_use]
    pub fn slot_types(&self) -> &[SlotType] {
        &self.slot_types
    }

    /// How many values the program leaves on the operand stack.
    ///
    /// ISO 32000-2 §7.10.5.3 requires this to *equal* the `Range`'s component count — "it
    /// shall be an error for the number of remaining operands to differ" — and
    /// [`Analysis::admits`] is where the two meet, before a frame.
    #[must_use]
    pub fn values_left(&self) -> u32 {
        self.values_left
    }

    /// Whether this program supplies exactly a `Range`'s components, and whether the
    /// `Range` itself is one a clamp can be written against.
    ///
    /// Separate from [`analyse`] because the `Range` belongs to the *shading*, not to the
    /// program: one uploaded program may serve two shadings, and §5 of the brief wants both
    /// questions answerable before a frame rather than during one.
    ///
    /// **Exactly**, per ISO 32000-2 §7.10.5.3: "it shall be an error for the number of
    /// remaining operands to *differ* from the number of output variables specified by
    /// Range". A program leaving more values than the `Range` declares is as much that
    /// error as one leaving fewer — reading the top three of five would draw a page the
    /// clause calls an error, and drawing it is what principle 6 prices above refusing it.
    ///
    /// # Errors
    ///
    /// [`FunctionRefusal::OutputCount`], [`FunctionRefusal::RangeNotFinite`] or
    /// [`FunctionRefusal::RangeNotOrdered`].
    pub fn admits(&self, range: FnRange) -> Result<(), FunctionRefusal> {
        validate_range(range)?;
        let required = range.components();
        let produced = usize::try_from(self.values_left).unwrap_or(usize::MAX);
        if produced != required {
            return Err(FunctionRefusal::OutputCount { produced, required });
        }
        Ok(())
    }

    /// The slots a `Range`'s components are read from, bottom component first.
    ///
    /// [`Analysis::admits`] has already required the two counts to be equal, so this is
    /// every slot the program leaves — the arithmetic is kept general because it is what
    /// *says* that, and because a `Range` that reached here without admission would name
    /// the top values rather than silently the wrong ones.
    pub(super) fn output_slots(&self, range: FnRange) -> Vec<u32> {
        let depth = self.values_left;
        let required = u32::try_from(range.components()).unwrap_or(0);
        let base = depth.saturating_sub(required);
        (base..depth).collect()
    }

    /// Whether an independent processor evaluation can be expected to agree.
    #[must_use]
    pub fn agreement(&self) -> Agreement {
        self.agreement
    }

    /// How many times the program pops a value the operand stack has not got.
    ///
    /// Non-zero means the program's picture depends on a decision ISO 32000-2 does not
    /// take — see [`Source::EmptyStackZero`](super::Source::EmptyStackZero). The count rather
    /// than a flag on purpose: it is
    /// the difference between "this program leans on it once" and the caller's own
    /// `pi_seven_segment.pdf`, which leans on it three times and has a whole dead branch as
    /// a result.
    #[must_use]
    pub fn empty_stack_pops(&self) -> u32 {
        self.empty_stack_pops
    }

    /// Whether the program's result depends on the empty-stack decision at all.
    #[must_use]
    pub fn relies_on_empty_stack(&self) -> bool {
        self.empty_stack_pops > 0
    }

    /// How many instructions the program has.
    #[must_use]
    pub fn program_length(&self) -> usize {
        self.program_length
    }

    /// How many bytes this analysis holds, for the resource budget a device charges an
    /// uploaded program against.
    ///
    /// Counted over the lowered tree rather than derived from the instruction count: a
    /// `Branch` holds two more lists, so the second number is not the first (CLAUDE.md
    /// principle 3 — a budget that priced only half of what it stores would be a budget
    /// in name).
    #[must_use]
    pub fn stored_bytes(&self) -> u64 {
        let slots = (self.slot_types.len() as u64).saturating_mul(size_of::<SlotType>() as u64);
        super::lowered::steps_bytes(&self.steps)
            .saturating_add(slots)
            .saturating_add(size_of::<Self>() as u64)
    }

    /// The identity of the *program*, which is what an upload deduplicates by.
    ///
    /// Computed by the walk rather than by [`generate`](super::generate) so that a pipeline
    /// cache can be asked its question before any WGSL exists — a lookup that had to
    /// generate the text first would not be a cache.
    #[must_use]
    pub fn program_hash(&self) -> ProgramHash {
        self.hash
    }

    /// The identity of the *shader* a `Range` generates from this program.
    ///
    /// Two ranges of different component counts produce two different functions from one
    /// program, so the pipeline cache keys on this and the upload table keys on
    /// [`Analysis::program_hash`].
    #[must_use]
    pub fn shader_hash(&self, range: FnRange) -> ProgramHash {
        super::hash::with_components(self.hash, range.components())
    }
}

/// Admit a program, or refuse it by name.
///
/// This is what `Device::upload_function` runs, and running it there rather than mid-encode
/// is the point: §5 of the brief wants a limit discoverable *before* the frame, and a caller
/// that learns its program is unsupported at upload can fall back before it has built a
/// scene at all.
///
/// # Errors
///
/// A [`FunctionRefusal`] naming the number or the operator that failed.
pub fn analyse(program: &[FnOp]) -> Result<Analysis, FunctionRefusal> {
    if program.len() > MAX_PROGRAM_LENGTH {
        return Err(FunctionRefusal::ProgramTooLong {
            length: program.len(),
            limit: MAX_PROGRAM_LENGTH,
        });
    }
    validate_jumps(program)?;

    let mut walk = Walk::new(program);
    let mut steps = Vec::new();
    walk.region(0, program.len(), 0, &mut steps)?;

    Ok(Analysis {
        steps,
        max_depth: slot_index(walk.max_depth),
        slot_types: walk.stack.iter().map(|cell| cell.ty).collect(),
        values_left: slot_index(walk.stack.len()),
        agreement: walk.approximate.unwrap_or(Agreement::Bounded),
        empty_stack_pops: walk.empty_stack_pops,
        program_length: program.len(),
        hash: super::hash::hash(program),
    })
}

/// ISO 32000-2 §7.10.1's output clip is a `clamp`, and WGSL's `clamp` returns the *high*
/// bound for every input when the two are the other way round — a uniform wrong colour that
/// looks like a colour. Table 38 requires `min <= max`; this is that requirement, checked
/// where the shader that depends on it is written rather than trusted from elsewhere.
fn validate_range(range: FnRange) -> Result<(), FunctionRefusal> {
    for (component, [min, max]) in range.bounds().iter().enumerate() {
        if !min.is_finite() || !max.is_finite() {
            return Err(FunctionRefusal::RangeNotFinite { component });
        }
        if min > max {
            return Err(FunctionRefusal::RangeNotOrdered {
                component,
                min: *min,
                max: *max,
            });
        }
    }
    Ok(())
}

/// A depth or slot position as the index the shader names it by.
///
/// Total because [`Walk::push`](super::walk::Walk::push) refuses at
/// [`MAX_OPERAND_SLOTS`](super::MAX_OPERAND_SLOTS), which is 64.
#[allow(
    clippy::cast_possible_truncation,
    reason = "the push budget bounds every depth at MAX_OPERAND_SLOTS, which is 64"
)]
pub(super) fn slot_index(depth: usize) -> u32 {
    depth as u32
}

/// Every jump target is strictly ahead of the jump and inside the program.
///
/// This is what bounds execution, and it is checked before the walk rather than during it
/// so that the walk's recursion cannot be driven by a target it has not validated.
///
/// # Errors
///
/// [`FunctionRefusal::BackwardJump`] or [`FunctionRefusal::JumpOutOfRange`].
pub fn validate_jumps(program: &[FnOp]) -> Result<(), FunctionRefusal> {
    for (at, op) in program.iter().enumerate() {
        let (FnOp::Jump { target } | FnOp::JumpUnless { target }) = *op else {
            continue;
        };
        let out_of_range = || FunctionRefusal::JumpOutOfRange {
            at,
            target,
            length: program.len(),
        };
        let index = usize::try_from(target).map_err(|_| out_of_range())?;
        if index <= at {
            return Err(FunctionRefusal::BackwardJump { at, target });
        }
        if index > program.len() {
            return Err(out_of_range());
        }
    }
    Ok(())
}
