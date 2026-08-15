//! A colour that is a *program*: ISO 32000-2 §7.10.5 type 4 functions as a paint.
//!
//! §8.7.4.5.2 defines a type 1 (function-based) shading as "a colour value for each
//! individual point" given by a mathematical function of two variables, and says of that
//! function:
//!
//! > The function need not be smooth or continuous.
//!
//! When the function is a §7.10.5 PostScript calculator program, the caller's only
//! device-independent option is to sample it onto a grid and hand us pixels — which is
//! what happens today, and costs that tree 1 142.8 ms of scene building for a single
//! page. This module is the other option: hand us the *program*, and let the device
//! evaluate it per fragment. `doc/adr/0053` is the decision and its evidence.
//!
//! # What this module is and is not
//!
//! It is **vocabulary only**. Nothing here analyses, validates or evaluates a program;
//! this crate cannot, because a scene knows nothing about a device (ADR 0001) and the
//! analysis exists to answer questions about a *shader*. `quorra_gpu::function` owns the
//! walk that admits or refuses a program, and the generator that lowers an admitted one.
//!
//! # Decisions taken elsewhere, which this vocabulary encodes
//!
//! - **`if`/`ifelse` never reach us.** The caller lowers them to [`FnOp::JumpUnless`] and
//!   [`FnOp::Jump`], so this crate never sees a `{}`. §7.10.5.1 admits procedures nowhere
//!   except as those two operators' operands, so nothing is lost.
//! - **Jumps are forward-only**, which is what makes a program's length a bound on its own
//!   execution. It is *validated* on the device side rather than trusted: a claim that
//!   bounds a loop is a claim a shader's safety rests on.
//! - **A literal carries its type.** Table 42's `not` is two operators wearing one name —
//!   logical negation on a boolean, one's complement on an integer — and an untyped
//!   literal cannot tell them apart, so [`FnOp::PushInt`] and [`FnOp::PushReal`] are
//!   separate.
//! - **Only `DeviceGray` and `DeviceRGB`.** [`FnRange`] carries the component count in its
//!   variant; a `DeviceCMYK` function is refused by name, because colour conversion is
//!   settled upstream (§4.5 of the brief) and CLAUDE.md's stack table forbids a
//!   colour-management crate here.

/// One instruction of a compiled ISO 32000-2 §7.10.5 type 4 (PostScript calculator)
/// function.
///
/// The operator set is Table 42 exactly, with three substitutions: `if` and `ifelse` are
/// [`FnOp::JumpUnless`]/[`FnOp::Jump`], `true` and `false` are [`FnOp::PushBool`], and the
/// numeric tokens Table 42 has no name for are [`FnOp::PushInt`]/[`FnOp::PushReal`].
///
/// §7.10.5.2 makes the *PostScript Language Reference, third edition* normative for these
/// operators' semantics; where a variant's meaning is not obvious from its name, its doc
/// comment says what that document requires of it.
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
    /// `abs`: absolute value.
    Abs,
    /// `add`: sum. Integer-preserving when both operands are integers.
    Add,
    /// `atan`: `num den atan`, the angle in **degrees** in `[0, 360)`.
    Atan,
    /// `ceiling`: the least integer not less than the operand.
    Ceiling,
    /// `cos`: cosine of an angle in **degrees**.
    Cos,
    /// `cvi`: convert to integer, truncating toward zero.
    Cvi,
    /// `cvr`: convert to real.
    Cvr,
    /// `div`: real division, "always a real number even if both operands are integers".
    Div,
    /// `exp`: `base exponent exp`. A negative base is meaningful only for an integer
    /// exponent.
    Exp,
    /// `floor`: the greatest integer not greater than the operand.
    Floor,
    /// `idiv`: integer division, truncating. Both operands must be integers.
    Idiv,
    /// `ln`: natural logarithm.
    Ln,
    /// `log`: common (base-10) logarithm.
    Log,
    /// `mod`: integer remainder, taking the sign of the dividend.
    Mod,
    /// `mul`: product. Integer-preserving when both operands are integers.
    Mul,
    /// `neg`: negation.
    Neg,
    /// `round`: the nearest integer; a tie goes to **the greater of the two**, so
    /// `-6.5 round` is `-6.0`. Neither Rust's nor WGSL's built-in rounding is this rule.
    Round,
    /// `sin`: sine of an angle in **degrees**.
    Sin,
    /// `sqrt`: square root of a non-negative operand.
    Sqrt,
    /// `sub`: difference. Integer-preserving when both operands are integers.
    Sub,
    /// `truncate`: the operand with its fractional part removed.
    Truncate,

    // Table 42, relational, boolean and bitwise.
    /// `and`: logical on two booleans, bitwise on two integers.
    And,
    /// `bitshift`: `int shift bitshift`, left for a positive shift and right for a
    /// negative one; bits shifted out are lost and bits shifted in are 0.
    Bitshift,
    /// `eq`: equality. Exact, with an integer and a real of the same value equal.
    Eq,
    /// `ge`: greater than or equal.
    Ge,
    /// `gt`: greater than.
    Gt,
    /// `le`: less than or equal.
    Le,
    /// `lt`: less than.
    Lt,
    /// `ne`: inequality.
    Ne,
    /// `not`: logical negation on a boolean, one's complement on an integer. Which of the
    /// two an occurrence means is a static property of its operand.
    Not,
    /// `or`: logical on two booleans, bitwise on two integers.
    Or,
    /// `xor`: logical on two booleans, bitwise on two integers.
    Xor,

    // Table 42, stack.
    /// `copy`: `n copy` duplicates the top *n* operands.
    Copy,
    /// `dup`: duplicate the top operand.
    Dup,
    /// `exch`: exchange the top two operands.
    Exch,
    /// `index`: `n index` copies the operand *n* below the top.
    Index,
    /// `pop`: discard the top operand.
    Pop,
    /// `roll`: `n j roll` rotates the top *n* operands by *j*.
    Roll,

    /// Pop a boolean; when it is false, continue at `target`. Forward only.
    JumpUnless {
        /// The instruction index to continue at. Strictly greater than this jump's own.
        target: u32,
    },
    /// Continue at `target`. Forward only.
    Jump {
        /// The instruction index to continue at. Strictly greater than this jump's own.
        target: u32,
    },
}

/// §7.10.1's `Range` for a function paint: `[min, max]` per output component, and the
/// component count with it.
///
/// One type rather than a `[f32; 6]` plus an `outputs: u32`, because those two can
/// disagree and this cannot. `DeviceCMYK` is deliberately absent: colour conversion is
/// settled upstream (§4.5), and CLAUDE.md's stack table forbids a colour-management
/// crate here.
///
/// The bounds are not decoration. ISO 32000-2 §7.10.1:
///
/// > Input values passed to the function shall be clipped to the domain, and output
/// > values produced by the function shall be clipped to the range.
///
/// The second half of that sentence is this type's, and it is normative. **The first half
/// is not a type 1 shading's**: §8.7.4.5.2 says of a point outside the transformed domain
/// rectangle that it "shall be painted with the shading's background colour (Background);
/// if the shading dictionary has no Background entry, such points shall be left
/// unpainted". A shading discards outside its domain; it does not clamp into it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FnRange {
    /// `DeviceGray`: one component.
    Gray([f32; 2]),
    /// `DeviceRGB`: three components, in order.
    Rgb([[f32; 2]; 3]),
}

impl FnRange {
    /// How many colour components this range declares. The variant *is* the count.
    #[must_use]
    pub fn components(self) -> usize {
        match self {
            Self::Gray(_) => 1,
            Self::Rgb(_) => 3,
        }
    }

    /// The bounds, one `[min, max]` pair per component.
    #[must_use]
    pub fn bounds(&self) -> &[[f32; 2]] {
        match self {
            Self::Gray(pair) => std::slice::from_ref(pair),
            Self::Rgb(triple) => triple,
        }
    }

    /// Whether every bound is finite and `min <= max` on every component.
    ///
    /// Table 38 requires the ordering, and an inverted pair is not a harmless oddity: the
    /// output clip is a `clamp`, and `clamp(v, high, low)` returns the *upper* bound for
    /// every input under WGSL's own definition — a uniform wrong colour that looks like a
    /// colour. Refused at the boundary, never normalised (§4.7 of the brief).
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.bounds()
            .iter()
            .all(|[min, max]| min.is_finite() && max.is_finite() && min <= max)
    }
}
