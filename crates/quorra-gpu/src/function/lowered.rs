//! The shape an admitted §7.10.5 program has after the walk, and before the WGSL.
//!
//! One responsibility: **vocabulary for a program whose every static question is already
//! answered.** A [`Step`] names the slots it reads and the slot it writes; a
//! [`Branch`](Step::Branch) is a tree rather than a jump target; the choice between Table
//! 42's two `not`s is already made. Nothing here decides anything — [`super::analyse`]
//! decided it all, and [`super::generate`] only prints.
//!
//! That seam is the reason this is a type rather than a `String` the walk appends to (as
//! the spike's `walk.rs` did): the analysis is testable without a generator, the generator
//! is testable without a walk, and the lowered form can be interpreted independently in a
//! test so the two can be checked against each other.
//!
//! # The slot model, and the one invariant the generated shader rests on
//!
//! **A slot index *is* an operand-stack position.** Slot 0 is the bottom of the stack, slot
//! *k* is the value *k* positions above it, and a program that reaches depth *d* uses slots
//! `0..d`. The generated shader declares one `var` per slot, so the count of them is
//! [`super::Analysis::max_depth`] and nothing larger.
//!
//! The alternative — a fresh single-assignment slot per computed value, so `dup`, `exch`
//! and `roll` become free renames and emit no code at all — is cleaner and is *not* taken.
//! It would declare one `var` per instruction: 482 of them for the caller's seven-segment
//! witness against 8, on a shader whose measured cold compile is 6.3 ms
//! (`doc/spike-function-paint.md` §3) and whose compile cost sits on the caller's
//! first-frame path. Trading a measured number for an unmeasured elegance is what
//! CLAUDE.md principle 2 forbids; this is the shape the spike measured.

use super::operators::{Binary, Unary};

/// Where a [`Step`] reads a value from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The operand-stack position of a value the program put there.
    Slot(u32),
    /// The zero a pop from an empty operand stack yields.
    ///
    /// ISO 32000-2 defines nothing here and PostScript would raise `stackunderflow`; the
    /// caller's evaluator serves it from `unwrap_or(0.0)` and their own
    /// `pi_seven_segment.pdf` depends on it three times, so this is a **deliberate choice**
    /// rather than a reading of the specification. [`super::Analysis::empty_stack_pops`]
    /// counts them so that the choice is reported and never adopted invisibly.
    EmptyStackZero,
}

/// One step of an admitted program, over the slot model this module's comment describes.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Put a literal in a slot. Integers and booleans arrive here already carried in `f32`,
    /// which is the operand stack's one representation; their *types* were consumed by the
    /// walk and survive only in the operators it chose.
    Literal {
        /// The slot written.
        slot: u32,
        /// The value.
        value: f32,
    },
    /// One-operand operator.
    Unary {
        /// The slot written.
        slot: u32,
        /// The operator.
        op: Unary,
        /// Where the operand comes from.
        operand: Source,
    },
    /// Two-operand operator, with `left` the deeper operand — `a b sub` gives
    /// `left = a, right = b`.
    Binary {
        /// The slot written.
        slot: u32,
        /// The operator.
        op: Binary,
        /// The deeper operand.
        left: Source,
        /// The shallower operand.
        right: Source,
    },
    /// A simultaneous assignment: **every source is read before any target is written.**
    ///
    /// That invariant is the whole reason this is one step rather than a list of moves.
    /// `exch` over slots 3 and 4 is `[(3, Slot(4)), (4, Slot(3))]`, and performing it as two
    /// sequential moves would leave both slots holding one value. The generator honours it
    /// by reading every source into a `let` first; a reader of the WGSL sees why the
    /// temporaries are there because this is the reason.
    ///
    /// `dup`, `exch`, `index`, `copy` and `roll` lower to this and to nothing else. A write
    /// whose source is its own slot is dropped by the walk rather than emitted, so a
    /// `0 index` or a `3 0 roll` costs no instruction at all.
    Permute {
        /// Target slot and where its new value comes from, in target order.
        writes: Vec<(u32, Source)>,
    },
    /// A structured conditional. An `if` has an empty `on_false`.
    Branch {
        /// The value tested. `on_true` runs when it is not zero.
        condition: Source,
        /// Steps taken when the condition holds.
        on_true: Vec<Step>,
        /// Steps taken when it does not.
        on_false: Vec<Step>,
    },
}

/// What the walk proved about the value in one operand-stack position.
///
/// ISO 32000-2 §7.10.5.1 gives the calculator three types — "expressions involving only
/// integers, real numbers, and boolean values" — and Table 42's `not` means different
/// operations on two of them. The operand stack itself carries one representation (`f32`),
/// so the type has to be a static fact or it is not a fact at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotType {
    /// A real number.
    Real,
    /// An integer.
    Integer,
    /// `true` or `false`.
    Boolean,
    /// No static type describes the value: either two arms of an `ifelse` left different
    /// types here, or it is the zero a pop from an empty operand stack yields — which the
    /// specification gives no type because it gives it no value either.
    Undecided,
}

impl SlotType {
    /// The type a slot has where two arms of an `ifelse` wrote different ones.
    ///
    /// `Integer` and `Real` merge to `Real`: §7.10.5.2's arithmetic operators accept
    /// either, and the merge loses nothing a real consumer needs. Anything merged with
    /// `Boolean` is [`SlotType::Undecided`], because `not` is the operator whose two
    /// meanings genuinely differ and a wrong guess there is a wrong colour.
    #[must_use]
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (a, b) if a == b => a,
            (Self::Integer, Self::Real) | (Self::Real, Self::Integer) => Self::Real,
            _ => Self::Undecided,
        }
    }

    /// The type as a refusal message spells it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Real => "a real",
            Self::Integer => "an integer",
            Self::Boolean => "a boolean",
            Self::Undecided => "a value of no decidable type",
        }
    }
}

/// How closely an independent processor evaluation of the same program can be expected to
/// agree with the device's — `doc/adr/0053` §3's classification.
///
/// The distinction is **not** which operators a program uses. It is whether an operator
/// whose two implementations may differ can reach one that turns a small difference into a
/// large one. `doc/research-function-paint-arithmetic.md` §3.4 states why there is no third
/// answer:
///
/// > Any inexact operator anywhere upstream of any comparison, branch or truncation makes
/// > the whole program's output arbitrary at the pixels where the two evaluations land on
/// > different sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agreement {
    /// No inexact operator's value reaches an amplifier, so the difference between the two
    /// evaluations stays a difference *of colour* — the currency ISO 32000-2 §10.7.3
    /// already measures a shading's accuracy in — rather than a difference of branch.
    ///
    /// **What this does not claim.** It is not bitwise identity, and the variant's name in
    /// ADR 0053 is stronger than the property. WGSL §15.7.5 lets an implementation
    /// reassociate and fuse the arithmetic of the straight-line expression a generated
    /// shader hands it; §15.7.4.1 gives `div` 2.5 ULP where IEEE 754 gives the host correct
    /// rounding; and ADR 0006's store rounding still sits between the shader and the texel
    /// (`doc/spike-function-paint.md` §5 measured 246 044 texels off by one from that step
    /// alone). The claim is that the disagreement stays *bounded and small*, which for a
    /// program with no amplifier in it is a property of the program rather than an
    /// observation about the two documents that happened to be measured.
    Exact,
    /// An inexact operator's value reaches an amplifier, so the two evaluations may differ
    /// by a whole colour over whatever set of points the program itself decides. There is no
    /// bound to state, so none is offered.
    Approximate {
        /// The inexact operator, as Table 42 spells it.
        inexact: &'static str,
        /// Where it is, as an index into the program.
        inexact_at: usize,
        /// The amplifier its value reaches, as Table 42 spells it — or `"if"` where the
        /// value is a conditional's own test.
        amplifier: &'static str,
        /// Where the amplifier is, as an index into the program.
        amplifier_at: usize,
    },
}
