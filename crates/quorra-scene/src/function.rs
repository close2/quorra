//! ISO 32000-2 §7.10.5 type 4 (PostScript calculator) functions, in compiled form.
//!
//! This module holds one thing: the vocabulary a §7.10.5 program is handed to us in,
//! and the §8.7.4.5.2 paint that carries one. It has no evaluator, no analyser and no
//! shader — a scene says *what* is to be drawn (`doc/adr/0001`), and how a program is
//! lowered is the device's business.
//!
//! # Where the semantics come from, and where they stop
//!
//! ISO 32000-2:2020 §7.10.5.2 lists the operators in "Table 42 — Operators in Type 4
//! functions" and then delegates their meaning:
//!
//! > The PostScript Language Reference, Third Edition shall define the semantics of
//! > these operators and all other syntax rules of the PostScript language. Although the
//! > semantics are those of the corresponding PostScript language operators, a full
//! > PostScript language compatible interpreter is not required.
//!
//! So PLRM3 is normative for every operator below, by way of clause 2's normative
//! reference; ISO 32000-2's own summary of the same operators is Annex B (informative),
//! and the one-line description quoted on each variant here is Annex B's. Where the two
//! differ in sharpness — `round`'s tie direction is the clearest case — Annex B is a
//! summary and PLRM3 is the requirement.
//!
//! **Neither document states a precision, a rounding mode or an accuracy requirement**
//! for evaluating a function; `doc/research-function-paint-arithmetic.md` §1.7 records
//! that silence with its scope, and `doc/adr/0053` is the decision taken under it.
//!
//! # What a compiled program is, and what bounds it
//!
//! The caller compiles the `{ … }` stream before we see it, so three of §7.10.5's
//! syntactic features are already gone: there are no braces, no procedure objects and
//! no nesting. `if` and `ifelse` arrive as [`FnOp::JumpUnless`] and [`FnOp::Jump`] over
//! a flat list, and **every jump is forward**, which is what makes the instruction count
//! an execution bound rather than a hope. That property is validated rather than
//! trusted.
//!
//! # The conformance corpus
//!
//! `crates/quorra-function-conformance` holds one program per Table 42 operator with
//! its expected value and the clause that value came from, the cases where the standard
//! defines nothing, and a reference evaluator written from the clause. Nothing here
//! evaluates anything; that crate is where a reading of PLRM3 becomes checkable.

/// One instruction of a compiled ISO 32000-2 §7.10.5 type 4 (PostScript calculator)
/// function.
///
/// The variant names are Table 42's operator names. Each carries the operator's
/// description from ISO 32000-2 Annex B (informative), because a reader of a compiled
/// program should not have to hold PLRM3 open to know what an opcode is; the *exact*
/// semantics, including every tie, sign and type rule, are PLRM3's and are pinned case
/// by case in `crates/quorra-function-conformance`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FnOp {
    /// A real literal.
    PushReal(f32),
    /// An integer literal. Separate from `PushReal` on purpose: Table 42's `not` is two
    /// operators wearing one name — logical negation on a boolean, one's complement on
    /// an integer — and an untyped literal cannot tell them apart.
    PushInt(i32),
    /// §7.10.5's `true` and `false`.
    PushBool(bool),

    // Table 42, arithmetic.
    /// `num₁ abs num₂` — "Return absolute value of *num₁*".
    Abs,
    /// `num₁ num₂ add sum` — "Return *num₁* plus *num₂*".
    Add,
    /// `num den atan angle` — "Return arc tangent of *num*/*den* in degrees".
    Atan,
    /// `num₁ ceiling num₂` — "Return ceiling of *num₁*".
    Ceiling,
    /// `angle cos real` — "Return cosine of *angle* degrees".
    Cos,
    /// `num cvi int` — "Convert to *integer*".
    Cvi,
    /// `num cvr real` — "Convert to *real*".
    Cvr,
    /// `num₁ num₂ div quotient` — "Return *num₁* divided by *num₂*".
    Div,
    /// `base exponent exp real` — "Raise base to exponent power".
    Exp,
    /// `num₁ floor num₂` — "Return floor of *num₁*".
    Floor,
    /// `int₁ int₂ idiv quotient` — "Return *int₁* divided by *int₂* as an integer".
    Idiv,
    /// `num ln real` — "Return natural logarithm (base *e*)".
    Ln,
    /// `num log real` — "Return common logarithm (base 10)".
    Log,
    /// `int₁ int₂ mod remainder` — "Return remainder after dividing *int₁* by *int₂*".
    Mod,
    /// `num₁ num₂ mul product` — "Return *num₁* times *num₂*".
    Mul,
    /// `num₁ neg num₂` — "Return negative of *num₁*".
    Neg,
    /// `num₁ round num₂` — "Round *num₁* to nearest integer". Annex B stops there;
    /// PLRM3 fixes the tie, and it is the one operator no host built-in implements.
    Round,
    /// `angle sin real` — "Return sine of *angle* degrees".
    Sin,
    /// `num sqrt real` — "Return square root of *num*".
    Sqrt,
    /// `num₁ num₂ sub difference` — "Return *num₁* minus *num₂*".
    Sub,
    /// `num₁ truncate num₂` — "Remove fractional part of *num₁*".
    Truncate,

    // Table 42, relational, boolean and bitwise.
    /// `bool₁|int₁ bool₂|int₂ and bool₃|int₃` — "Perform logical|bitwise and".
    And,
    /// `int₁ shift bitshift int₂` — "Perform bitwise shift of *int₁* (positive is
    /// left)".
    Bitshift,
    /// `any₁ any₂ eq bool` — "Test equal".
    Eq,
    /// `num₁ num₂ ge bool` — "Test greater than or equal".
    Ge,
    /// `num₁ num₂ gt bool` — "Test greater than".
    Gt,
    /// `num₁ num₂ le bool` — "Test less than or equal".
    Le,
    /// `num₁ num₂ lt bool` — "Test less than".
    Lt,
    /// `any₁ any₂ ne bool` — "Test not equal".
    Ne,
    /// `bool₁|int₁ not bool₂|int₂` — "Perform logical|bitwise not". The two operators
    /// wearing one name; which one an occurrence means is decided by the type of what
    /// reaches it, which is why a literal carries its type.
    Not,
    /// `bool₁|int₁ bool₂|int₂ or bool₃|int₃` — "Perform logical|bitwise inclusive or".
    Or,
    /// `bool₁|int₁ bool₂|int₂ xor bool₃|int₃` — "Perform logical|bitwise exclusive or".
    Xor,

    // Table 42, stack.
    /// `any₁ … anyₙ n copy any₁ … anyₙ any₁ … anyₙ` — "Duplicate top *n* elements".
    Copy,
    /// `any dup any any` — "Duplicate top element".
    Dup,
    /// `any₁ any₂ exch any₂ any₁` — "Exchange top two elements".
    Exch,
    /// `anyₙ … any₀ n index anyₙ … any₀ anyₙ` — "Duplicate arbitrary element".
    Index,
    /// `any pop –` — "Discard top element".
    Pop,
    /// `anyₙ₋₁ … any₀ n j roll …` — "Roll *n* elements up *j* times".
    Roll,

    /// Pop a boolean; when it is false, continue at `target`. Forward only.
    JumpUnless {
        /// The instruction index to continue at. Strictly greater than this
        /// instruction's own index, and at most the program's length — a target equal
        /// to the length halts.
        target: u32,
    },
    /// Continue at `target`. Forward only.
    Jump {
        /// As [`FnOp::JumpUnless::target`].
        target: u32,
    },
}

/// ISO 32000-2 §8.7.4.5.2's type 1 shading: a colour that is a function of position.
///
/// The function is a §7.10.5 program rather than a sampled grid, which is the whole
/// point: `doc/adr/0053` measured the caller's grid at 1 142.8 ms of scene building for
/// one page and the same arithmetic on the device at 0.060 ms, and the grid has to be
/// rebuilt at every zoom step where the program does not.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionPaint {
    /// The compiled program. Flat, with forward-only jumps, so its length bounds its
    /// own execution.
    pub program: std::sync::Arc<[FnOp]>,
    /// §8.7.4.5.2's `Domain`, as `[x_min, x_max, y_min, y_max]`.
    ///
    /// Table 38's `Domain` row: "Input values outside the declared domain shall be
    /// clipped to the nearest boundary value." The clip happens before the program
    /// runs, and it is normative — see [`FnRange`] for the other end of it.
    pub domain: [f32; 4],
    /// §8.7.4.5.2's `Matrix`: the shading's own space → scene space. Deliberately not
    /// the command's transform, for the reason `Paint::Shading::transform` already
    /// states.
    pub matrix: crate::geom::Affine,
    /// §7.10's `Range`, which also *is* the number of output components.
    pub range: FnRange,
}

/// §7.10's `Range` for a function paint: `[min, max]` per output component, and the
/// component count with it.
///
/// One type rather than a `[f32; 6]` plus an `outputs: u32`, because those two can
/// disagree and this cannot. `DeviceCMYK` is deliberately absent: colour conversion is
/// settled upstream (§4.5), and CLAUDE.md's stack table forbids a colour-management
/// crate here.
///
/// ISO 32000-2 §7.10.1, on why this is not advisory:
///
/// > Input values passed to the function shall be clipped to the domain, and output
/// > values produced by the function shall be clipped to the range.
///
/// The clause's own EXAMPLE is a function of range `[0 100]` whose output of −14 "is
/// replaced with 0, the nearest value in the defined range". A device that skips the
/// clip is wrong on every document whose range is not the unit interval, and right on
/// both of the caller's witnesses — which is why it is a test's job.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FnRange {
    /// `DeviceGray`: one component.
    Gray([f32; 2]),
    /// `DeviceRGB`: three components, in order.
    Rgb([[f32; 2]; 3]),
}

impl FnRange {
    /// How many colour components the program must leave on the stack.
    ///
    /// §7.10.5.3: "It shall be an error for the number of remaining operands to differ
    /// from the number of output variables specified by **Range** or for any of them to
    /// be objects other than numbers."
    #[must_use]
    pub const fn components(self) -> usize {
        match self {
            Self::Gray(_) => 1,
            Self::Rgb(_) => 3,
        }
    }

    /// The `[min, max]` pair for output component `index`, or `None` when the range has
    /// no such component.
    #[must_use]
    pub fn bounds(self, index: usize) -> Option<[f32; 2]> {
        match self {
            Self::Gray(pair) => (index == 0).then_some(pair),
            Self::Rgb(pairs) => pairs.get(index).copied(),
        }
    }
}
