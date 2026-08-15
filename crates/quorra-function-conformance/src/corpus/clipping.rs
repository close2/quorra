//! §7.10's two clips and §7.10.5.3's output count — the three normative rules that
//! belong to no operator.
//!
//! ISO 32000-2 §7.10.1, verbatim:
//!
//! > Each function definition includes a domain, the set of valid values for the input.
//! > Some types of functions also define a range, the set of valid values for the output.
//! > Input values passed to the function shall be clipped to the domain, and output
//! > values produced by the function shall be clipped to the range.
//!
//! Table 38 says the same twice more, once per entry: "Input values outside the declared
//! domain shall be clipped to the nearest boundary value" and "Output values outside the
//! declared range shall be clipped to the nearest boundary value."
//!
//! **This is the family an operator-by-operator corpus misses.** Both of the caller's
//! witnesses declare `/Domain [0 1 0 1]` and `/Range [0 1 0 1 0 1]`, so a device that
//! skipped either clip would draw both of them correctly and fail on the first document
//! whose range is not the unit interval. Two of the cases below are the clause's own
//! worked EXAMPLEs, transcribed as programs.

use crate::case::{Case, PsError, Subject};
use quorra_scene::function::{FnOp as Op, FnRange};

/// The clause's own example range, `[0 100]`, which is not the unit interval and
/// therefore not a range either witness would have exercised.
const RANGE_0_100: FnRange = FnRange::Gray([0.0, 100.0]);

/// A range wide enough to hold the domain examples' outputs without clipping, so that
/// each case tests one clip rather than two.
const RANGE_0_10: FnRange = FnRange::Gray([0.0, 10.0]);

/// Every case in this family.
pub const CASES: &[Case] = &[
    // ---- the domain clip ----------------------------------------------------
    Case::exact(
        "domain/clause-example-above-the-upper-bound",
        Subject::DomainClip,
        &[Op::PushInt(2), Op::Add],
        &[3.0],
        "ISO 32000-2 §7.10.1's own EXAMPLE, transcribed: \"Suppose the following \
         function is defined with a domain of [-1 1]. If the function is called with the \
         input value 6, that value is replaced with the nearest value in the defined \
         domain, 1, before the function is evaluated; the resulting output value is \
         therefore 3\", for f(x) = x + 2.",
    )
    .with_inputs(&[6.0])
    .with_domain([-1.0, 1.0, -1.0, 1.0])
    .with_range(RANGE_0_10),
    Case::exact(
        "domain/below-the-lower-bound",
        Subject::DomainClip,
        &[Op::PushInt(2), Op::Add],
        &[1.0],
        "The same function and domain as the clause's EXAMPLE, at the other end: \
         ISO 32000-2 Table 38's `Domain` row says inputs outside the domain \"shall be \
         clipped to \
         the nearest boundary value\", and the nearest boundary to −6 is −1, so \
         f = 1. Derived from the row; the clause works only the upper end.",
    )
    .with_inputs(&[-6.0])
    .with_domain([-1.0, 1.0, -1.0, 1.0])
    .with_range(RANGE_0_10),
    Case::exact(
        "domain/inside-passes-through",
        Subject::DomainClip,
        &[Op::PushInt(2), Op::Add],
        &[2.5],
        "The same function; an input already inside the domain is not moved. The clip is \
         a clamp and not a normalisation — nothing in §7.10.1 or Table 38 maps the \
         domain onto anything.",
    )
    .with_inputs(&[0.5])
    .with_domain([-1.0, 1.0, -1.0, 1.0])
    .with_range(RANGE_0_10),
    Case::exact(
        "domain/applies-to-the-second-input-too",
        Subject::DomainClip,
        &[Op::Exch, Op::Pop],
        &[1.0],
        "ISO 32000-2 Table 38's `Domain` row is written per input: \"For each i from 0 to m - 1, \
         Domain₂ᵢ shall be less than or equal to Domain₂ᵢ₊₁, and the iᵗʰ input value, xᵢ, \
         shall lie in the interval Domain₂ᵢ ≤ xᵢ ≤ Domain₂ᵢ₊₁.\" A shading's second input \
         is clipped against the second pair, so y = 9 becomes 1; the program discards x \
         and returns y. A device that clips only x draws this as 9.",
    )
    .with_inputs(&[0.0, 9.0])
    .with_domain([0.0, 1.0, 0.0, 1.0])
    .with_range(RANGE_0_10),
    // ---- the range clip -----------------------------------------------------
    Case::exact(
        "range/clause-example-below-the-lower-bound",
        Subject::RangeClip,
        &[Op::Exch, Op::PushInt(3), Op::Mul, Op::Add],
        &[0.0],
        "ISO 32000-2 §7.10.1's second EXAMPLE, transcribed: \"if the following function \
         is defined with a range of [0 100], and if the input values -6 and 4 are passed \
         to the function (and are within its domain), then the output value produced by \
         the function, -14, is replaced with 0, the nearest value in the defined range\", \
         for f(x₀, x₁) = 3 × x₀ + x₁.",
    )
    .with_inputs(&[-6.0, 4.0])
    .with_domain([-10.0, 10.0, -10.0, 10.0])
    .with_range(RANGE_0_100),
    Case::exact(
        "range/above-the-upper-bound",
        Subject::RangeClip,
        &[Op::PushReal(150.0)],
        &[100.0],
        "ISO 32000-2 Table 38's `Range` row: \"Output values outside the declared range shall be \
         clipped to the nearest boundary value.\" The upper end of the clause's own \
         `[0 100]`; the EXAMPLE works only the lower one.",
    )
    .with_range(RANGE_0_100),
    Case::exact(
        "range/each-component-has-its-own-bounds",
        Subject::RangeClip,
        &[Op::PushReal(5.0), Op::PushReal(5.0), Op::PushReal(5.0)],
        &[1.0, 5.0, 0.0],
        "ISO 32000-2 Table 38's `Range` row is written per component: \"for each j from 0 to n - 1, \
         Range₂ⱼ shall be less than or equal to Range₂ⱼ₊₁, and the jᵗʰ output value, yⱼ, \
         shall lie in the interval Range₂ⱼ ≤ yⱼ ≤ Range₂ⱼ₊₁\". Three identical outputs \
         against three different pairs give three different answers, which is what a \
         single shared clamp cannot do.",
    )
    .with_range(FnRange::Rgb([[0.0, 1.0], [0.0, 10.0], [-1.0, 0.0]])),
    // ---- the output count ---------------------------------------------------
    Case::error(
        "output-count/more-values-than-components",
        Subject::OutputCount,
        &[Op::PushInt(1), Op::PushInt(2)],
        PsError::OutputCount,
        "ISO 32000-2 §7.10.5.3: \"It shall be an error for the number of remaining \
         operands to differ from the number of output variables specified by Range\". A \
         one-component `Range` and two values left behind is that error, and it is the \
         clause's `shall`, not a convention.",
    ),
    Case::error(
        "output-count/a-boolean-is-not-an-output",
        Subject::OutputCount,
        &[Op::PushBool(true)],
        PsError::OutputCount,
        "ISO 32000-2 §7.10.5.3, the same sentence's second half: it is an error \"for any \
         of them to be objects other than numbers\". The count is right here and the type \
         is not.",
    ),
];
