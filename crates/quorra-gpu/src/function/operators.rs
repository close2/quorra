//! ISO 32000-2 Table 42's operators, split by arity, and the two properties of each that
//! decide whether a program can be trusted to agree with an independent evaluation.
//!
//! One responsibility: **the operator vocabulary and its arithmetic classification.** No
//! walking, no emitting. The classification is a table because it is a table — the
//! evidence for each row is `doc/research-function-paint-arithmetic.md` §3.2, and the two
//! predicates below are the only thing `doc/adr/0053` §3's decision reads.
//!
//! # The two properties, and why neither alone is dangerous
//!
//! - [`Unary::is_inexact`] / [`Binary::is_inexact`] — the two sides may compute different
//!   values from the same input, because WGSL declines to specify the operation as tightly
//!   as IEEE 754 does. Nothing here is a *bug*; every row is a deliberate concession WGSL
//!   §15.7.4.1 makes to what GPUs do.
//! - [`Unary::amplifies`] / [`Binary::amplifies`] — the operator is exact but
//!   discontinuous, so a difference of one unit in the last place upstream becomes a
//!   difference of order one downstream.
//!
//! §3.1 of the research states the composition rule this file exists to serve:
//!
//! > No operator is dangerous on its own. `ge` is exact; so is `truncate`. The danger is
//! > always a composition: an inexact operator upstream of an amplifier.

/// A Table 42 operator taking one operand.
///
/// `not` is absent by name and present twice: Table 42's `not` is two operators wearing
/// one name — logical negation on a boolean, one's complement on an integer — and which
/// one an occurrence means is a static property of its operand that
/// [`super::analyse`] resolves. That resolution is the whole reason a literal carries its
/// type in [`FnOp`](quorra_scene::FnOp).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unary {
    /// `abs`.
    Abs,
    /// `ceiling`.
    Ceiling,
    /// `cos`, of an angle in degrees.
    Cos,
    /// `cvi`.
    Cvi,
    /// `cvr`.
    Cvr,
    /// `floor`.
    Floor,
    /// `ln`.
    Ln,
    /// `log`.
    Log,
    /// `neg`.
    Neg,
    /// `round`, with PLRM3's half-toward-greater tie.
    Round,
    /// `sin`, of an angle in degrees.
    Sin,
    /// `sqrt`.
    Sqrt,
    /// `truncate`.
    Truncate,
    /// `not` over a boolean: logical negation.
    LogicalNot,
    /// `not` over an integer: one's complement.
    BitwiseNot,
}

impl Unary {
    /// The operator as ISO 32000-2 Table 42 spells it.
    #[must_use]
    pub fn table_42(self) -> &'static str {
        match self {
            Self::Abs => "abs",
            Self::Ceiling => "ceiling",
            Self::Cos => "cos",
            Self::Cvi => "cvi",
            Self::Cvr => "cvr",
            Self::Floor => "floor",
            Self::Ln => "ln",
            Self::Log => "log",
            Self::Neg => "neg",
            Self::Round => "round",
            Self::Sin => "sin",
            Self::Sqrt => "sqrt",
            Self::Truncate => "truncate",
            Self::LogicalNot | Self::BitwiseNot => "not",
        }
    }

    /// The `function_ops.wgsl` function that implements it.
    pub(crate) fn wgsl(self) -> &'static str {
        match self {
            Self::Abs => "ps_abs",
            Self::Ceiling => "ps_ceiling",
            Self::Cos => "ps_cos",
            Self::Cvi => "ps_cvi",
            Self::Cvr => "ps_cvr",
            Self::Floor => "ps_floor",
            Self::Ln => "ps_ln",
            Self::Log => "ps_log",
            Self::Neg => "ps_neg",
            Self::Round => "ps_round",
            Self::Sin => "ps_sin",
            Self::Sqrt => "ps_sqrt",
            Self::Truncate => "ps_truncate",
            Self::LogicalNot => "ps_not",
            Self::BitwiseNot => "ps_bit_not",
        }
    }

    /// Whether a conformant WGSL implementation and a conformant host may compute
    /// different values from the same input.
    ///
    /// `sin` and `cos`: WGSL §15.7.4.1 bounds the absolute error by 2⁻¹¹ only over
    /// `[-π, π]`, and "the accuracy is undefined for input values outside that range" —
    /// while §7.10.5.3's own worked example evaluates `sin` at ±360°. `ln`/`log`: absolute
    /// 2⁻²¹ inside `[0.5, 2.0]` and 3 ULP outside, against a correctly-rounded host libm.
    /// `sqrt`: "inherited from `1.0 / inverseSqrt(x)`", a 2 ULP reciprocal square root
    /// followed by a 2.5 ULP division, where IEEE 754 requires `sqrt` correctly rounded.
    #[must_use]
    pub fn is_inexact(self) -> bool {
        matches!(
            self,
            Self::Cos | Self::Ln | Self::Log | Self::Sin | Self::Sqrt
        )
    }

    /// Whether the operator turns a difference of one unit in the last place into a
    /// difference of order one.
    ///
    /// The five rounding operators are step functions: two evaluations that straddle an
    /// integer by a last bit land a whole unit apart. `not` over an integer truncates its
    /// operand first and so does the same; `not` over a *boolean* does not, because its
    /// operand is already exactly 0 or 1 and has no last bit to disagree about.
    #[must_use]
    pub fn amplifies(self) -> bool {
        match self {
            Self::Ceiling
            | Self::Cvi
            | Self::Floor
            | Self::Round
            | Self::Truncate
            | Self::BitwiseNot => true,
            Self::Abs
            | Self::Cos
            | Self::Cvr
            | Self::Ln
            | Self::Log
            | Self::Neg
            | Self::Sin
            | Self::Sqrt
            | Self::LogicalNot => false,
        }
    }
}

/// A Table 42 operator taking two operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binary {
    /// `add`.
    Add,
    /// `atan`.
    Atan,
    /// `div`.
    Div,
    /// `exp`.
    Exp,
    /// `idiv`.
    Idiv,
    /// `mod`.
    Mod,
    /// `mul`.
    Mul,
    /// `sub`.
    Sub,
    /// `and`.
    And,
    /// `bitshift`.
    Bitshift,
    /// `eq`.
    Eq,
    /// `ge`.
    Ge,
    /// `gt`.
    Gt,
    /// `le`.
    Le,
    /// `lt`.
    Lt,
    /// `ne`.
    Ne,
    /// `or`.
    Or,
    /// `xor`.
    Xor,
}

impl Binary {
    /// The operator as ISO 32000-2 Table 42 spells it.
    #[must_use]
    pub fn table_42(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Atan => "atan",
            Self::Div => "div",
            Self::Exp => "exp",
            Self::Idiv => "idiv",
            Self::Mod => "mod",
            Self::Mul => "mul",
            Self::Sub => "sub",
            Self::And => "and",
            Self::Bitshift => "bitshift",
            Self::Eq => "eq",
            Self::Ge => "ge",
            Self::Gt => "gt",
            Self::Le => "le",
            Self::Lt => "lt",
            Self::Ne => "ne",
            Self::Or => "or",
            Self::Xor => "xor",
        }
    }

    /// The `function_ops.wgsl` function that implements it.
    pub(crate) fn wgsl(self) -> &'static str {
        match self {
            Self::Add => "ps_add",
            Self::Atan => "ps_atan",
            Self::Div => "ps_div",
            Self::Exp => "ps_exp",
            Self::Idiv => "ps_idiv",
            Self::Mod => "ps_mod",
            Self::Mul => "ps_mul",
            Self::Sub => "ps_sub",
            Self::And => "ps_and",
            Self::Bitshift => "ps_bitshift",
            Self::Eq => "ps_eq",
            Self::Ge => "ps_ge",
            Self::Gt => "ps_gt",
            Self::Le => "ps_le",
            Self::Lt => "ps_lt",
            Self::Ne => "ps_ne",
            Self::Or => "ps_or",
            Self::Xor => "ps_xor",
        }
    }

    /// Whether a conformant WGSL implementation and a conformant host may compute
    /// different values from the same inputs.
    ///
    /// `div`: WGSL §15.7.4.1 allows 2.5 ULP where IEEE 754 requires correct rounding.
    /// `atan`: 4 096 ULP, which after conversion to Table 42's degrees is 0.028° — three
    /// orders of magnitude worse than any other row, and not a last-bit problem at all.
    /// `exp`: PLRM3 defines `−9 −1 exp ⇒ −0.111111`, a negative base, so it cannot be
    /// WGSL's `pow` and must be composed from `exp2` and `log2` — the error of two loose
    /// rows.
    ///
    /// `add`, `sub` and `mul` are *not* on this list, and that is a narrower claim than it
    /// looks: WGSL §15.7.5 permits an implementation to reassociate and fuse them, and a
    /// generated shader is exactly the shape that hands it a whole expression tree to do it
    /// over. They are excluded because the difference that licence permits is a rounding of
    /// the same magnitude as the operation's own, where the rows above are bounded by
    /// thousands of ULP or by nothing.
    #[must_use]
    pub fn is_inexact(self) -> bool {
        matches!(self, Self::Atan | Self::Div | Self::Exp)
    }

    /// Whether this operator turns a difference of one unit in the last place into a
    /// difference of order one — the property [`Agreement`](super::Agreement) turns on.
    ///
    /// The six relational operators are the obvious ones and the ones the caller named.
    /// The rest earn the label the same way: `idiv`, `mod`, `bitshift`, `and`, `or` and
    /// `xor` all truncate their operands to integers first, and a truncation at an integer
    /// boundary converts a last-bit disagreement into a whole unit.
    #[must_use]
    pub fn amplifies(self) -> bool {
        match self {
            Self::Idiv
            | Self::Mod
            | Self::And
            | Self::Bitshift
            | Self::Or
            | Self::Xor
            | Self::Eq
            | Self::Ge
            | Self::Gt
            | Self::Le
            | Self::Lt
            | Self::Ne => true,
            Self::Add | Self::Atan | Self::Div | Self::Exp | Self::Mul | Self::Sub => false,
        }
    }
}
