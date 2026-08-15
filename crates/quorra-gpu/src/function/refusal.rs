//! Why a §7.10.5 program is declined, by name.
//!
//! `RENDER_LIBRARY.md` §5, and CLAUDE.md principle 6: a backend may refuse, it may not
//! silently draw nothing — and a limit that exists is discoverable *before* the frame.
//! Every variant here names the number or the operator that failed, and every one is
//! reached by a program in `tests/function_refusals.rs`, because **a ground nobody can
//! reach is not a ground**.
//!
//! The `Domain` rectangle, the `Matrix` and the `Background` are **not** here, and their
//! absence follows the same rule from the other side: they belong to the *shading* rather
//! than to the program, they are a `Rect`, an `Affine` and a `Color` like every other one in
//! a scene, and `Rect::is_ordered`/`Affine::is_finite`/`Color::is_valid` are the checks the
//! scene boundary already applies to all of them. A second copy here would be a second
//! definition of a valid rectangle.
//!
//! Three of the spike's six grounds (`doc/spike-function-paint.md` §6) are absent, and
//! their absence is the point: "an operator outside Table 42", "unbalanced braces" and "a
//! procedure that is not an `if`/`ifelse` operand" were grounds because the spike compiled
//! PostScript *text*. We are handed [`FnOp`](quorra_scene::FnOp), a closed enum with no
//! procedure in it, so none of the three is expressible. The caller's compiler owns them.

use thiserror::Error;

/// What a program does that this backend cannot draw as asked.
///
/// A refusal is not a defect report about the document: several of these are perfectly
/// legal §7.10.5 programs that a *generated shader* cannot express (a `roll` whose count
/// is computed, an `ifelse` whose arms leave different depths). The caller's answer to any
/// of them is the same — fall back to the raster it builds today — which is why the
/// discrimination that matters is by name rather than by severity.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum FunctionRefusal {
    /// The program is longer than the stated budget. The generated shader's length is
    /// linear in the program's, and a shader nobody can compile is a frame nobody can
    /// draw; §7.10.5 states no limit, so this one is ours (see [`MAX_PROGRAM_LENGTH`](super::MAX_PROGRAM_LENGTH)).
    #[error("the program has {length} instructions, over the stated budget of {limit}")]
    ProgramTooLong {
        /// The program's length.
        length: usize,
        /// [`MAX_PROGRAM_LENGTH`](super::MAX_PROGRAM_LENGTH).
        limit: usize,
    },
    /// A jump whose target is not strictly ahead of it. Forward-only jumping is what makes
    /// a program's length a bound on its own execution, and it is validated rather than
    /// trusted because a loop's bound is a claim a shader's safety rests on.
    #[error("the jump at {at} targets {target}, which is not ahead of it")]
    BackwardJump {
        /// Index of the offending jump.
        at: usize,
        /// The target it named.
        target: u32,
    },
    /// A jump past the end of the program.
    #[error("the jump at {at} targets {target}, past the program's {length} instructions")]
    JumpOutOfRange {
        /// Index of the offending jump.
        at: usize,
        /// The target it named.
        target: u32,
        /// The program's length.
        length: usize,
    },
    /// A jump that does not nest: an unconditional jump reached in sequence, or a jump out
    /// of the arm that contains it. §7.10.5.1 admits only `if` and `ifelse`, both of which
    /// nest, so a jump graph that does not is one no compiled form should have produced —
    /// and a generated shader has no `goto` to lower it to.
    #[error("the jump at {at} does not nest as an `if` or an `ifelse`")]
    UnstructuredControlFlow {
        /// Index of the offending jump.
        at: usize,
    },
    /// `if`/`ifelse` nested deeper than the stated budget. The walk that admits a program
    /// recurses once per nesting level; §7.10.5 states no limit, so this one is ours
    /// (see [`MAX_BRANCH_NESTING`](super::MAX_BRANCH_NESTING)).
    #[error("`if`/`ifelse` at {at} nests deeper than the stated budget of {limit}")]
    BranchesTooDeep {
        /// Index of the conditional that exceeded the budget.
        at: usize,
        /// [`MAX_BRANCH_NESTING`](super::MAX_BRANCH_NESTING).
        limit: usize,
    },
    /// The operand stack would grow past the stated budget. A generated shader names one
    /// `var` per slot, so the count has to be known and bounded before the shader exists
    /// (see [`MAX_OPERAND_SLOTS`](super::MAX_OPERAND_SLOTS)); both of the caller's witnesses need 8.
    #[error("the operand stack would reach {needed} slots, over the stated budget of {limit}")]
    StackTooDeep {
        /// The depth the program reaches.
        needed: usize,
        /// [`MAX_OPERAND_SLOTS`](super::MAX_OPERAND_SLOTS).
        limit: usize,
    },
    /// `copy`, `index` or `roll` whose count is not a literal the walk can resolve. A
    /// generated shader cannot name a slot it cannot compute, and a walk that cannot
    /// resolve the count cannot state the program's own depth either.
    #[error("`{operator}` has a count that is not statically known")]
    DynamicStackCount {
        /// The stack operator, spelled as Table 42 spells it.
        operator: &'static str,
    },
    /// `copy`, `index` or `roll` whose count is statically known and out of range. PLRM3
    /// makes each of these a `rangecheck`; the pinned "a pop from an empty stack yields 0"
    /// covers a *pop*, not a count that names operands the stack never had.
    #[error("`{operator}` names {count} operands with {depth} on the stack")]
    StackCountOutOfRange {
        /// The stack operator, spelled as Table 42 spells it.
        operator: &'static str,
        /// The count it asked for.
        count: i64,
        /// The depth available where it asked.
        depth: usize,
    },
    /// A literal that is NaN or infinite. §7.10.1 makes a function a map between numbers;
    /// §4.7 of the brief refuses these loudly rather than letting them become NaN geometry —
    /// and WGSL's Finite Math Assumption would turn one into an indeterminate value, which
    /// is an arbitrary colour that looks like a colour.
    #[error("the literal at {at} is {value}, which is not a number")]
    NonFiniteLiteral {
        /// Index of the offending instruction.
        at: usize,
        /// The value as given.
        value: f32,
    },
    /// The two arms of an `ifelse` leave different operand-stack depths, so no static
    /// assignment of values to slots describes the join.
    #[error(
        "the `ifelse` at {at} leaves {then_depth} operands on one arm and {else_depth} on the other"
    )]
    UnbalancedBranches {
        /// Index of the conditional.
        at: usize,
        /// Depth after the arm taken when the condition is true.
        then_depth: usize,
        /// Depth after the other arm.
        else_depth: usize,
    },
    /// A type-sensitive operator reached an operand of the wrong static type.
    ///
    /// Table 42's `not`, `and`, `or` and `xor` mean different operations on a boolean and
    /// on an integer, and PLRM3 requires integers of `idiv`, `mod` and `bitshift`. Lowering
    /// one of them over a real would silently truncate — a plausible-looking wrong colour,
    /// which CLAUDE.md principle 6 prices above a refusal.
    #[error("`{operator}` was given {found}, and requires {required}")]
    OperandType {
        /// The operator, spelled as Table 42 spells it.
        operator: &'static str,
        /// What its operands must be.
        required: &'static str,
        /// What the walk proved they are.
        found: &'static str,
    },
    /// A type-sensitive operator reached a value whose static type is not decidable —
    /// either two arms of an `ifelse` left different types in the slot, or the value is the
    /// zero a pop from an empty operand stack yields, which has no type at all.
    #[error(
        "`{operator}` reached a value whose type two arms disagreed about, or which came from an empty operand stack"
    )]
    UndecidableOperandType {
        /// The operator, spelled as Table 42 spells it.
        operator: &'static str,
    },
    /// The program leaves a different number of values than its `Range` declares
    /// components. ISO 32000-2 §7.10.5.3, verbatim:
    ///
    /// > It shall be an error for the number of remaining operands to differ from the
    /// > number of output variables specified by **Range** or for any of them to be objects
    /// > other than numbers.
    ///
    /// **Differ**, in both directions, and the second direction is the one that is easy to
    /// get wrong: a program leaving three values under a one-component `Range` is as much
    /// this error as one leaving none, and reading the top value while discarding two would
    /// draw a page the clause calls an error. The first direction would paint the missing
    /// components with the empty-stack zero, which is a black-or-primary-coloured page that
    /// looks like a page.
    ///
    /// Static, and therefore discoverable before the frame:
    /// [`Analysis::admits`](super::Analysis::admits) is the question, and a caller can ask
    /// it with no device in hand.
    #[error("the program leaves {produced} values; its Range declares {required} components")]
    OutputCount {
        /// How many values the program leaves.
        produced: usize,
        /// What [`FnRange`](quorra_scene::FnRange) requires.
        required: usize,
    },
    /// A `Range` bound that is NaN or infinite.
    #[error("the Range's component {component} has a non-finite bound")]
    RangeNotFinite {
        /// Which output component, counting from zero.
        component: usize,
    },
    /// A `Range` component whose minimum exceeds its maximum.
    ///
    /// Refused for the reason the scene boundary refuses an inverted rectangle: clamping
    /// into `[max, min]` returns the upper bound for *every* input, which is a flat colour
    /// that looks drawn.
    #[error("the Range's component {component} is {min} to {max}, which is not an interval")]
    RangeNotOrdered {
        /// Which output component, counting from zero.
        component: usize,
        /// The lower bound as given.
        min: f32,
        /// The upper bound as given.
        max: f32,
    },
}
