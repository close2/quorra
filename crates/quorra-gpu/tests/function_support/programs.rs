#![allow(
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects,
    reason = "the sample grid is small integer arithmetic over a fixed range"
)]

//! The programs every function test is run against. **Tests only.**
//!
//! Small on purpose, and each one exists to reach something: an operator family, a control
//! shape, a classification, or a clamp. They are not a conformance corpus — a sibling owns
//! that — they are the witnesses this module's own claims are made on, and each carries a
//! sentence saying which claim.
//!
//! Every jump target below is written out by hand and checked by
//! `function::validate_jumps` inside `analyse`, so a mistake here is a refused program
//! rather than a silently different one.

use quorra_scene::{Color, FnOp, FnRange, Point, Rect};

/// A program with the shading parameters a test evaluates it under.
///
/// The parameters are *not* part of the program any more (revision 2 of the pinned
/// vocabulary): a program is uploaded and a shading names it with a domain, a matrix, a range
/// and a background. This carries the shading half so a test can state both at once.
pub struct Witness {
    /// What the program is for, used in assertion messages.
    pub name: &'static str,
    /// The program.
    pub program: Vec<FnOp>,
    /// §8.7.4.5.2's `Domain`, in the shading's own space.
    pub domain: Rect,
    /// §7.10.1's `Range`.
    pub range: FnRange,
    /// §8.7.4.5.2's `Background`. `None` leaves points outside the domain unpainted.
    pub background: Option<Color>,
}

/// The unit square, which is what both of the caller's real witnesses declare.
#[must_use]
pub fn unit_domain() -> Rect {
    Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0))
}

/// A witness over the unit domain with no background.
#[must_use]
pub fn witness(program: &[FnOp], range: FnRange) -> Witness {
    Witness {
        name: "unnamed",
        program: program.to_vec(),
        domain: unit_domain(),
        range,
        background: None,
    }
}

/// `DeviceRGB` over the unit interval, which is what a shading's `Range` usually is.
pub const UNIT_RGB: FnRange = FnRange::Rgb([[0.0, 1.0], [0.0, 1.0], [0.0, 1.0]]);
/// `DeviceGray` over the unit interval.
pub const UNIT_GRAY: FnRange = FnRange::Gray([0.0, 1.0]);
/// A range wide enough that a program's own numbers survive the clip, for the tests that are
/// about arithmetic rather than about clipping.
pub const WIDE_GRAY: FnRange = FnRange::Gray([-1000.0, 1000.0]);
/// The same, in three components.
pub const WIDE_RGB: FnRange = FnRange::Rgb([[-1000.0, 1000.0]; 3]);

/// `pop pop 0.25` in `DeviceGray`: the smallest program that draws anything.
///
/// The two pops are not decoration. ISO 32000-2 §7.10.5.3 makes it an error for the number
/// of values left to *differ* from the `Range`'s component count, so a one-component
/// program has to consume the two inputs a §8.7.4.5.2 shading pushes.
pub const CONSTANT_GREY: &[FnOp] = &[FnOp::Pop, FnOp::Pop, FnOp::PushReal(0.25)];

/// `x`, `y`, `x*y` in `DeviceRGB` — the program that proves the two inputs arrive in the two
/// slots the walk reserved, and that `index` reaches past the top of the stack.
pub const COORDINATES: &[FnOp] = &[
    FnOp::PushInt(1),
    FnOp::Index,
    FnOp::PushInt(1),
    FnOp::Index,
    FnOp::Mul,
];

/// `add`, `dup`, `neg`, `abs`, `mul` and nothing else: the classification's `Exact` case with
/// no inexact operator anywhere in it.
pub const ARITHMETIC_ONLY: &[FnOp] = &[
    FnOp::Add,
    FnOp::Dup,
    FnOp::Dup,
    FnOp::Neg,
    FnOp::Abs,
    FnOp::PushReal(0.5),
    FnOp::Mul,
];

/// `x 0.5 ge { 1 0 0 } { 0 0 1 } ifelse` — a discontinuity, but reached from no inexact
/// operator, so still `Exact`.
pub const DISCONTINUOUS: &[FnOp] = &[
    FnOp::Pop,
    FnOp::PushReal(0.5),
    FnOp::Ge,
    FnOp::JumpUnless { target: 8 },
    FnOp::PushReal(1.0),
    FnOp::PushReal(0.0),
    FnOp::PushReal(0.0),
    FnOp::Jump { target: 11 },
    FnOp::PushReal(0.0),
    FnOp::PushReal(0.0),
    FnOp::PushReal(1.0),
];

/// `x sqrt` — an inexact operator whose value reaches nothing that amplifies it.
pub const SQRT_ALONE: &[FnOp] = &[FnOp::Pop, FnOp::Sqrt];

/// `x sqrt 0.5 ge` — the same operator, one comparison later, and therefore `Approximate`.
pub const SQRT_INTO_COMPARISON: &[FnOp] = &[FnOp::Pop, FnOp::Sqrt, FnOp::PushReal(0.5), FnOp::Ge];

/// `63 not` — Table 42's *other* `not`, which PLRM3 makes `-64` and the caller's own
/// evaluator makes `0.0`.
pub const NOT_ON_INTEGER: &[FnOp] = &[FnOp::Pop, FnOp::Pop, FnOp::PushInt(63), FnOp::Not];

/// `x 0.5 ge not` — the logical reading, chosen from the same instruction.
pub const NOT_ON_BOOLEAN: &[FnOp] = &[FnOp::Pop, FnOp::PushReal(0.5), FnOp::Ge, FnOp::Not];

/// `dup 3 1 roll 2 copy exch pop pop` — every stack operator except `index`, over slots the
/// generated shader has to permute rather than rename.
pub const STACK_SHUFFLE: &[FnOp] = &[
    FnOp::Dup,
    FnOp::PushInt(3),
    FnOp::PushInt(1),
    FnOp::Roll,
    FnOp::PushInt(2),
    FnOp::Copy,
    FnOp::Exch,
    FnOp::Pop,
    FnOp::Pop,
];

/// Pops three values off a stack that holds two, then adds to the zero the third pop
/// yielded: the pinned empty-stack decision, twice.
pub const EMPTY_STACK: &[FnOp] = &[
    FnOp::Pop,
    FnOp::Pop,
    FnOp::Pop,
    FnOp::PushReal(1.0),
    FnOp::Add,
];

/// `17 5 idiv 17 5 mod and 1 3 bitshift or 6 xor` — the integer family, whose operands the
/// walk has to prove are integers before any of it can be lowered.
pub const INTEGER_OPS: &[FnOp] = &[
    FnOp::Pop,
    FnOp::Pop,
    FnOp::PushInt(17),
    FnOp::PushInt(5),
    FnOp::Idiv,
    FnOp::PushInt(17),
    FnOp::PushInt(5),
    FnOp::Mod,
    FnOp::And,
    FnOp::PushInt(1),
    FnOp::PushInt(3),
    FnOp::Bitshift,
    FnOp::Or,
    FnOp::PushInt(6),
    FnOp::Xor,
];

/// `sin`, `cos` and `exp`, none of them reaching an amplifier.
pub const TRANSCENDENTAL_A: &[FnOp] = &[
    FnOp::Pop,
    FnOp::Dup,
    FnOp::Sin,
    FnOp::Exch,
    FnOp::Cos,
    FnOp::PushReal(2.0),
    FnOp::PushReal(3.0),
    FnOp::Exp,
];

/// `ln`, `log` and `atan`, over operands their domains admit.
pub const TRANSCENDENTAL_B: &[FnOp] = &[
    FnOp::Pop,
    FnOp::PushReal(1.0),
    FnOp::Add,
    FnOp::Dup,
    FnOp::Ln,
    FnOp::Exch,
    FnOp::Log,
    FnOp::PushReal(1.0),
    FnOp::PushReal(2.0),
    FnOp::Atan,
];

/// `-6.5 round`, `2.5 round`, `-1.5 floor` — the three values PLRM3's own `round` entry and
/// the two built-ins disagree about.
pub const ROUNDING: &[FnOp] = &[
    FnOp::Pop,
    FnOp::Pop,
    FnOp::PushReal(-6.5),
    FnOp::Round,
    FnOp::PushReal(2.5),
    FnOp::Round,
    FnOp::PushReal(-1.5),
    FnOp::Floor,
];

/// An `ifelse` inside an `ifelse`: the shape the walk recovers from four jumps.
pub const NESTED_BRANCHES: &[FnOp] = &[
    FnOp::Pop,
    FnOp::PushReal(0.5),
    FnOp::Gt,
    FnOp::JumpUnless { target: 12 },
    FnOp::PushReal(0.25),
    FnOp::PushReal(0.75),
    FnOp::Gt,
    FnOp::JumpUnless { target: 10 },
    FnOp::PushReal(1.0),
    FnOp::Jump { target: 11 },
    FnOp::PushReal(0.5),
    FnOp::Jump { target: 13 },
    FnOp::PushReal(0.0),
];

/// Leaves `5.0`, `-3.0` and `0.5`, none of which the ranges below admit unchanged. The
/// witness for §7.10's output clamp.
pub const OUT_OF_RANGE: &[FnOp] = &[
    FnOp::Pop,
    FnOp::Pop,
    FnOp::PushReal(5.0),
    FnOp::PushReal(-3.0),
    FnOp::PushReal(0.5),
];

/// A `Range` that is not the unit interval on any component, and not the same on any two.
///
/// Both of the caller's witnesses declare `/Range [0 1 0 1 0 1]`, so a defect in the output
/// clamp is invisible on their corpus and on the spike's two programs. This is the shape
/// that has to be a unit test rather than a corpus run.
pub const ODD_RGB: FnRange = FnRange::Rgb([[0.2, 0.8], [-1.0, 1.0], [10.0, 20.0]]);

/// Every witness, with a range each is meaningful under.
#[must_use]
pub fn all() -> Vec<Witness> {
    let named: &[(&'static str, &[FnOp], FnRange)] = &[
        ("constant grey", CONSTANT_GREY, UNIT_GRAY),
        ("coordinates", COORDINATES, UNIT_RGB),
        ("arithmetic only", ARITHMETIC_ONLY, WIDE_RGB),
        ("discontinuous threshold", DISCONTINUOUS, UNIT_RGB),
        ("sqrt alone", SQRT_ALONE, WIDE_GRAY),
        ("sqrt into comparison", SQRT_INTO_COMPARISON, UNIT_GRAY),
        ("not on integer", NOT_ON_INTEGER, WIDE_GRAY),
        ("not on boolean", NOT_ON_BOOLEAN, UNIT_GRAY),
        ("stack shuffle", STACK_SHUFFLE, WIDE_RGB),
        ("empty stack", EMPTY_STACK, WIDE_GRAY),
        ("integer ops", INTEGER_OPS, WIDE_GRAY),
        ("transcendental a", TRANSCENDENTAL_A, WIDE_RGB),
        ("transcendental b", TRANSCENDENTAL_B, WIDE_RGB),
        ("rounding", ROUNDING, WIDE_RGB),
        ("nested branches", NESTED_BRANCHES, UNIT_GRAY),
        ("out of range", OUT_OF_RANGE, ODD_RGB),
    ];
    named
        .iter()
        .map(|(name, program, range)| Witness {
            name,
            ..witness(program, *range)
        })
        .collect()
}

/// The points every witness is evaluated at.
///
/// The grid runs from -0.25 to 0.75 on both axes, so a quarter of it is **outside** the unit
/// domain: §8.7.4.5.2's discard is only observable from there, and a grid that stayed inside
/// would agree with the wrong reading as happily as with the right one.
#[must_use]
pub fn sample_points() -> Vec<(f32, f32)> {
    let mut points = Vec::new();
    for row in 0..7 {
        for column in 0..7 {
            points.push(((column as f32) / 6.0 - 0.25, (row as f32) / 6.0 - 0.25));
        }
    }
    points
}

/// A witness whose domain is a quarter of the area the sample grid covers, with a background.
///
/// The shape §8.7.4.5.2's rule needs and the caller's corpus cannot supply: both of their
/// witnesses declare `/Domain [0 1 0 1]`, so a shader that clamped instead of discarding
/// would agree with one that discards on every page they have.
#[must_use]
pub fn small_domain(background: Option<Color>) -> Witness {
    Witness {
        name: "small domain",
        program: COORDINATES.to_vec(),
        domain: Rect::new(Point::new(0.0, 0.0), Point::new(0.5, 0.5)),
        range: UNIT_RGB,
        background,
    }
}
