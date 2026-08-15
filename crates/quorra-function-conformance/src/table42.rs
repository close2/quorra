//! ISO 32000-2 Table 42, as a type.
//!
//! The table lists 42 operator *names* and nothing else — 21 arithmetic, 13
//! relational/boolean/bitwise, 2 conditional, 6 stack — and delegates their meaning to
//! PLRM3. Its one-line descriptions live in ISO's own Annex B (informative), which is
//! what `quorra_scene::function::FnOp` quotes.
//!
//! This enumeration exists so that "every operator has a case" is a test rather than a
//! reviewer's count: [`Table42::ALL`] is iterated by the gate's
//! `the_corpus_covers_every_table_42_operator`, and an operator added here without a
//! case fails it.
//!
//! It is deliberately **not** `FnOp`, and the difference is the lowering. Table 42 has
//! `if`, `ifelse`, `true` and `false`; a compiled program has `JumpUnless`, `Jump` and
//! `PushBool` instead, plus two literal variants Table 42 has no operators for. Deriving
//! this list from `FnOp` would lose exactly the four operators whose lowering is the
//! thing worth testing.

/// One of the 42 operators of ISO 32000-2 Table 42.
///
/// The count is the table's own: 21 arithmetic, 13 relational/boolean/bitwise, 2
/// conditional, 6 stack; see the module comment for why this is not `FnOp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Table42 {
    /// `abs`.
    Abs,
    /// `add`.
    Add,
    /// `atan`.
    Atan,
    /// `ceiling`.
    Ceiling,
    /// `cos`.
    Cos,
    /// `cvi`.
    Cvi,
    /// `cvr`.
    Cvr,
    /// `div`.
    Div,
    /// `exp`.
    Exp,
    /// `floor`.
    Floor,
    /// `idiv`.
    Idiv,
    /// `ln`.
    Ln,
    /// `log`.
    Log,
    /// `mod`.
    Mod,
    /// `mul`.
    Mul,
    /// `neg`.
    Neg,
    /// `round`.
    Round,
    /// `sin`.
    Sin,
    /// `sqrt`.
    Sqrt,
    /// `sub`.
    Sub,
    /// `truncate`.
    Truncate,
    /// `and`.
    And,
    /// `bitshift`.
    Bitshift,
    /// `eq`.
    Eq,
    /// `false`.
    False,
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
    /// `not`.
    Not,
    /// `or`.
    Or,
    /// `true`.
    True,
    /// `xor`.
    Xor,
    /// `if`, which reaches us lowered to [`FnOp::JumpUnless`].
    If,
    /// `ifelse`, which reaches us lowered to [`FnOp::JumpUnless`] and [`FnOp::Jump`].
    Ifelse,
    /// `copy`.
    Copy,
    /// `dup`.
    Dup,
    /// `exch`.
    Exch,
    /// `index`.
    Index,
    /// `pop`.
    Pop,
    /// `roll`.
    Roll,
}

impl Table42 {
    /// Every operator in Table 42, in the table's own reading order.
    pub const ALL: [Self; 42] = [
        Self::Abs,
        Self::Add,
        Self::Atan,
        Self::Ceiling,
        Self::Cos,
        Self::Cvi,
        Self::Cvr,
        Self::Div,
        Self::Exp,
        Self::Floor,
        Self::Idiv,
        Self::Ln,
        Self::Log,
        Self::Mod,
        Self::Mul,
        Self::Neg,
        Self::Round,
        Self::Sin,
        Self::Sqrt,
        Self::Sub,
        Self::Truncate,
        Self::And,
        Self::Bitshift,
        Self::Eq,
        Self::False,
        Self::Ge,
        Self::Gt,
        Self::Le,
        Self::Lt,
        Self::Ne,
        Self::Not,
        Self::Or,
        Self::True,
        Self::Xor,
        Self::If,
        Self::Ifelse,
        Self::Copy,
        Self::Dup,
        Self::Exch,
        Self::Index,
        Self::Pop,
        Self::Roll,
    ];

    /// The operator's name as Table 42 spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Abs => "abs",
            Self::Add => "add",
            Self::Atan => "atan",
            Self::Ceiling => "ceiling",
            Self::Cos => "cos",
            Self::Cvi => "cvi",
            Self::Cvr => "cvr",
            Self::Div => "div",
            Self::Exp => "exp",
            Self::Floor => "floor",
            Self::Idiv => "idiv",
            Self::Ln => "ln",
            Self::Log => "log",
            Self::Mod => "mod",
            Self::Mul => "mul",
            Self::Neg => "neg",
            Self::Round => "round",
            Self::Sin => "sin",
            Self::Sqrt => "sqrt",
            Self::Sub => "sub",
            Self::Truncate => "truncate",
            Self::And => "and",
            Self::Bitshift => "bitshift",
            Self::Eq => "eq",
            Self::False => "false",
            Self::Ge => "ge",
            Self::Gt => "gt",
            Self::Le => "le",
            Self::Lt => "lt",
            Self::Ne => "ne",
            Self::Not => "not",
            Self::Or => "or",
            Self::True => "true",
            Self::Xor => "xor",
            Self::If => "if",
            Self::Ifelse => "ifelse",
            Self::Copy => "copy",
            Self::Dup => "dup",
            Self::Exch => "exch",
            Self::Index => "index",
            Self::Pop => "pop",
            Self::Roll => "roll",
        }
    }
}
