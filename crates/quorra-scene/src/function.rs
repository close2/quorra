//! A colour that is a small program: ISO 32000-2 §7.10.5's type 4 function, compiled.
//!
//! One responsibility: the vocabulary a §8.7.4.5.2 type 1 (function-based) shading is
//! written in, and the paint that carries it. Nothing here evaluates a program and
//! nothing here generates a shader — [`FunctionPaint::check`] is the boundary's question
//! ("is this well-formed data at all"), and the device-side analysis that follows it (stack
//! depth, the static classification of the operators that cannot be trusted across
//! adapters, the resolution of `copy`/`index`/`roll` counts) belongs to `quorra-gpu`,
//! where ADR 0053 puts it.
//!
//! # Why this exists at all
//!
//! §8.7.4.5.2, on the type 1 shading:
//!
//! > In Type 1 (function-based) shadings, the colour at every point in the domain is
//! > defined by a specified mathematical function. The function need not be smooth or
//! > continuous.
//!
//! The caller evaluates that function once per device pixel on the processor, which for
//! a page whose whole content is one such shading costs them 1 142.8 ms of scene
//! building. Evaluating it in a generated fragment shader costs 0.060 ms of device time
//! for the same page. ADR 0053 is that decision and its evidence;
//! `doc/spike-function-paint.md` is the measurement and
//! `doc/research-function-paint-arithmetic.md` the clause work.
//!
//! # The compiled form, and what it is not
//!
//! We never see PostScript source. §7.10.5.1 admits `{ … }` procedures only as the
//! operands of `if` and `ifelse`, and the caller lowers both to the two jump
//! instructions below before we see them, so a [`FunctionPaint::program`] is **flat**.
//! Jumps are forward-only, and that is checked rather than trusted: it is the single
//! property that makes a program's length a bound on its own execution, which is what a
//! fragment shader without a loop needs.
//!
//! [`FnOp`] is closed over Table 42's forty-two operators, and the mapping is exact:
//! the twenty-one arithmetic and the eleven relational, boolean and bitwise operators
//! are one variant each; Table 42's `true` and `false` are [`FnOp::PushBool`]; its `if`
//! and `ifelse` are [`FnOp::JumpUnless`] and [`FnOp::Jump`]; its six stack operators are
//! one variant each. 21 + 11 + 2 + 2 + 6 = 42. The three `Push*` variants are literals,
//! which Table 42 does not list because PostScript source does not need an operator to
//! write one down.
//!
//! # Whose semantics these are
//!
//! §7.10.5.2, verbatim:
//!
//! > "Table 42 — Operators in Type 4 functions" lists the operators that can be used in
//! > this type of function. The PostScript Language Reference, Third Edition shall
//! > define the semantics of these operators and all other syntax rules of the
//! > PostScript language. Although the semantics are those of the corresponding
//! > PostScript language operators, a full PostScript language compatible interpreter is
//! > not required.
//!
//! So PLRM3 is normative here by reference, and three operators below are documented
//! *against PLRM3* because a host built-in of the same name is a different function:
//! [`FnOp::Round`] (ties toward the greater, so `-6.5 round` is `-6.0` — neither Rust's
//! `f32::round` nor WGSL's `round`), [`FnOp::Atan`] (two operands, degrees, `0..360`)
//! and [`FnOp::Exp`] (`base exponent exp`, not `e^x`). Each says so on its own doc.
//!
//! # What the specification does *not* say
//!
//! It states no precision, no rounding mode and no accuracy requirement for the
//! evaluation of a function of any type: §7.3.3 defers to "the internal representations
//! used in the computer on which the PDF processor is running", and PLRM3, asked the
//! same question, defers to the hardware again. That silence is recorded in full in
//! `doc/research-function-paint-arithmetic.md` §1.7, and it is why ADR 0053 refuses a
//! program that can reach a transcendental on a path into a comparison instead of
//! promising a tolerance nobody can derive.

mod validate;

pub use validate::MAX_PROGRAM_LENGTH;

/// One instruction of a compiled ISO 32000-2 §7.10.5 type 4 (PostScript calculator)
/// function.
///
/// Every variant except the three literals and the two jumps is one operator of
/// §7.10.5.2's Table 42, whose semantics that clause delegates to PLRM3. Where a
/// variant's name collides with a host built-in that means something else, the doc says
/// which one PLRM3 requires.
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
    /// `num abs` — the absolute value of the operand, keeping its type.
    Abs,
    /// `num1 num2 add` — their sum.
    Add,
    /// `num den atan` — PLRM3: "returns the angle (in degrees between 0 and 360) whose
    /// tangent is num divided by den. Either num or den may be 0, but not both."
    ///
    /// **Two operands, degrees, and a `0..360` range.** A host `atan2` takes two
    /// operands and returns radians in `-π..=π`; a host `atan` takes one. Neither is
    /// this operator, and `0 0 atan` is an error PLRM3 names (`undefinedresult`) where
    /// Rust's `f32::atan2` quietly returns `0.0`.
    Atan,
    /// `num1 ceiling` — the least integer not less than the operand.
    Ceiling,
    /// `angle cos` — PLRM3: "returns the cosine of angle, which is interpreted as an
    /// angle in degrees". **Degrees, not radians.**
    Cos,
    /// `num cvi` — PLRM3: "If the operand is a real number, it truncates any fractional
    /// part (that is, rounds it toward 0) and converts it to an integer."
    Cvi,
    /// `num cvr` — convert to real. A no-op on a value already real.
    Cvr,
    /// `num1 num2 div` — PLRM3: "divides num1 by num2, producing a result that is always
    /// a real number even if both operands are integers". A zero divisor is PLRM3's
    /// `undefinedresult`.
    Div,
    /// `base exponent exp` — PLRM3: "raises base to the exponent power … If the exponent
    /// has a fractional part, the result is meaningful only if the base is nonnegative."
    ///
    /// **Not `e^x`.** PLRM3's own example is `-9 -1 exp ⇒ -0.111111`, a negative base,
    /// which no `pow` built on `exp2(y · log2(x))` can produce.
    Exp,
    /// `num1 floor` — the greatest integer not greater than the operand.
    Floor,
    /// `int1 int2 idiv` — integer division truncated toward zero; both operands are
    /// integers and so is the result. A zero divisor is PLRM3's `undefinedresult`.
    Idiv,
    /// `num ln` — the natural logarithm. A non-positive operand is PLRM3's `rangecheck`.
    Ln,
    /// `num log` — the common (base 10) logarithm. A non-positive operand is PLRM3's
    /// `rangecheck`.
    Log,
    /// `int1 int2 mod` — the remainder of `int1 idiv int2`, where PLRM3 fixes the tie a
    /// reader would otherwise have to guess: the "sign of the result is the same as the
    /// sign of the dividend". A zero divisor is `undefinedresult`.
    Mod,
    /// `num1 num2 mul` — their product.
    Mul,
    /// `num neg` — the negative of the operand.
    Neg,
    /// `num1 round` — PLRM3: "returns the integer value nearest to num1. If num1 is
    /// equally close to its two nearest integers, round returns the greater of the two."
    ///
    /// **Half toward the greater, which is neither of the two obvious built-ins.**
    /// PLRM3's own example is `-6.5 round ⇒ -6.0`. Rust's `f32::round` is half away from
    /// zero and gives `-7`; WGSL's `round` is half to even and gives `-6` here but `-8`
    /// for `-7.5`, where PLRM3 requires `-7`. A lowering that reaches for either built-in
    /// is wrong on half the ties, so both sides spell the rule out.
    Round,
    /// `angle sin` — PLRM3: "returns the sine of angle, which is interpreted as an angle
    /// in degrees". **Degrees, not radians**; §7.10.5.3's own `DoubleDot` example
    /// evaluates it at ±360°.
    Sin,
    /// `num sqrt` — PLRM3: "returns the square root of num, which must be a nonnegative
    /// number". A negative operand is `rangecheck`.
    Sqrt,
    /// `num1 num2 sub` — their difference, `num1 − num2`.
    Sub,
    /// `num1 truncate` — PLRM3: "truncates num1 toward 0 by removing its fractional
    /// part".
    Truncate,

    // Table 42, relational, boolean and bitwise.
    /// `bool1 bool2 and` or `int1 int2 and` — logical conjunction on booleans, bitwise
    /// AND on integers. Which one it is follows from the operands' types, which is why
    /// [`FnOp::PushInt`] and [`FnOp::PushBool`] are separate instructions.
    And,
    /// `int1 shift bitshift` — PLRM3: "shifts the binary representation of int1 left by
    /// shift bits and returns the result. Bits shifted out are lost; bits shifted in are
    /// 0. If shift is negative, a right shift by –shift bits is performed."
    ///
    /// PLRM3, as we read it, does not state what a shift past the operand's width does;
    /// `doc/research-function-paint-arithmetic.md` §5 records that as unverified rather
    /// than settled.
    Bitshift,
    /// `any1 any2 eq` — equality. PLRM3 defines it on values, with numeric coercion:
    /// "an integer and a real number representing the same mathematical value are
    /// considered equal by **eq**". There is no tolerance in the clause and none here.
    Eq,
    /// `num1 num2 ge` — whether the first is greater than or equal to the second.
    Ge,
    /// `num1 num2 gt` — whether the first is strictly greater.
    Gt,
    /// `num1 num2 le` — whether the first is less than or equal to the second.
    Le,
    /// `num1 num2 lt` — whether the first is strictly less.
    Lt,
    /// `any1 any2 ne` — the negation of [`FnOp::Eq`].
    Ne,
    /// `bool not` or `int not` — logical negation on a boolean, **one's complement on an
    /// integer**. The two are different functions sharing a name, and `63 not` is `-64`,
    /// not `false`: an evaluator that implements only the logical one is wrong on every
    /// integer operand.
    Not,
    /// `bool1 bool2 or` or `int1 int2 or` — inclusive disjunction, or bitwise OR.
    Or,
    /// `bool1 bool2 xor` or `int1 int2 xor` — exclusive disjunction, or bitwise XOR.
    Xor,

    // Table 42, stack.
    /// `any1 … anyn n copy` — push a copy of the top `n` elements, in order. `n` is
    /// itself popped from the stack, which is why ADR 0053 requires it to be statically
    /// resolvable: a generated shader cannot name a slot it cannot compute.
    Copy,
    /// `any dup` — push a second copy of the top element.
    Dup,
    /// `any1 any2 exch` — exchange the top two elements.
    Exch,
    /// `anyn … any0 n index` — push a copy of the element `n` places below the top,
    /// counting the top as 0. `n` comes off the stack; see [`FnOp::Copy`].
    Index,
    /// `any pop` — discard the top element.
    Pop,
    /// `anyn₋₁ … any0 n j roll` — circularly shift the top `n` elements by `j` places,
    /// positive `j` moving each element toward the top. Both counts come off the stack;
    /// see [`FnOp::Copy`].
    Roll,

    /// Pop a boolean; when it is false, continue at `target`. Forward only.
    JumpUnless {
        /// Index into [`FunctionPaint::program`], strictly greater than this
        /// instruction's own and at most the program's length — where the length itself
        /// means "stop", which is how a trailing `if` lowers.
        target: u32,
    },
    /// Continue at `target`. Forward only.
    Jump {
        /// Index into [`FunctionPaint::program`], under the same rule as
        /// [`FnOp::JumpUnless`]'s.
        target: u32,
    },
}

/// ISO 32000-2 §8.7.4.5.2's type 1 shading: a colour that is a function of position.
///
/// Held behind an `Arc` in [`Paint::Function`](crate::paint::Paint::Function) because a
/// program is shared: one page-wide shading painted through many marks is one program,
/// one upload and — the number that decides the design — one generated shader.
///
/// # What a device does with it, in order
///
/// 1. Map the fragment's position back through [`FunctionPaint::matrix`] into the
///    shading's own space. That the matrix is invertible is checked here, not there;
///    see [`FunctionPaint::check`].
/// 2. Decide whether that position is inside [`FunctionPaint::domain`]. §8.7.4.5.2 is
///    explicit that the domain rectangle is a *region*, not a clamp on the position:
///
///    > Points within the shading's bounding box (`BBox`) that fall outside this
///    > transformed domain rectangle shall be painted with the shading's background
///    > colour (`Background`); if the shading dictionary has no `Background` entry, such
///    > points shall be left unpainted.
///
///    This paint carries no background colour, so a device outside the domain rectangle
///    paints nothing there. `doc/notes-function-scene.md` records this as the one open
///    contract question: a caller with a `/Background` has no way to say so yet.
/// 3. Run the program at that position, and clip each result into
///    [`FunctionPaint::range`]. §7.10.1's second clip is not optional:
///
///    > Input values passed to the function shall be clipped to the domain, and output
///    > values produced by the function shall be clipped to the range.
///
/// The clipping and the discard are the generator's to emit; what this crate owns is that
/// the numbers they work against are well-formed.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionPaint {
    /// The compiled program. Flat, with forward-only jumps, so its length bounds its
    /// own execution.
    pub program: std::sync::Arc<[FnOp]>,
    /// §8.7.4.5.2's `Domain`, as `[x_min, x_max, y_min, y_max]`.
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
/// # Why the pairs are not required to be `[0, 1]`
///
/// They are a *clip*, not a colour claim. §8.7.4.5.2's Table 78, `Function` row, applies
/// a second adjustment afterwards — "If the value returned by the function for a given
/// colour component is out of range, it shall be adjusted to the nearest valid value" —
/// so a document may legitimately declare a range wider than the colour space and rely
/// on that. Refusing anything but the unit interval would refuse a conforming file, so
/// the only conditions here are the two a clip cannot do without: finite, and ordered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FnRange {
    /// `DeviceGray`: one component.
    Gray([f32; 2]),
    /// `DeviceRGB`: three components, in order.
    Rgb([[f32; 2]; 3]),
}

impl FnRange {
    /// The `[min, max]` pairs, in component order — one for
    /// [`Gray`](FnRange::Gray), three for [`Rgb`](FnRange::Rgb).
    ///
    /// Borrowed rather than copied so that the two variants can be checked by one loop
    /// without either arm having to name the other's length.
    #[must_use]
    pub fn pairs(&self) -> &[[f32; 2]] {
        match self {
            Self::Gray(pair) => std::slice::from_ref(pair),
            Self::Rgb(pairs) => pairs,
        }
    }

    /// How many colour components the program leaves on the stack: 1 for
    /// [`Gray`](FnRange::Gray), 3 for [`Rgb`](FnRange::Rgb).
    #[must_use]
    pub fn components(&self) -> usize {
        self.pairs().len()
    }
}
