#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    reason = "every expected value here is an exact number a clause states, so an epsilon \
              would weaken the assertion into one that three different rounding rules pass; \
              and the helpers are test scaffolding whose panic is the test failing"
)]

//! The lowering, checked against an independent evaluation — and the operators and clauses
//! whose values the specification states outright.
//!
//! Two different jobs, kept apart on purpose:
//!
//! - **The lowering.** A stack machine over `&[FnOp]` and a slot machine over
//!   `Analysis::steps()` are two ways to run the same program. They share the arithmetic
//!   deliberately, so a disagreement is a defect in the slot allocation, the
//!   reads-before-writes invariant, the branch structure recovered from the jumps, or the
//!   resolution of Table 42's two `not`s — and in nothing else. The *arithmetic* is what
//!   `function_device.rs` checks, against the shader.
//! - **The values.** Where PLRM3 or ISO 32000-2 states what an operator returns or what a
//!   shading paints, the expected value below is that statement, quoted in the test's own
//!   comment. That is the direction CLAUDE.md principle 5 requires: not "this is what the
//!   other implementation produces", but "this is what the clause says".

mod function_support;

use function_support::programs::{self, WIDE_GRAY, Witness};
use function_support::{lowered, reference};
use quorra_gpu::function::analyse;
use quorra_scene::{Color, FnOp, FnRange};

/// Every witness, at every sample point, through both machines.
#[test]
fn the_lowered_form_computes_what_the_program_does() {
    for witness in programs::all() {
        let analysis = analyse(&witness.program).unwrap();
        for (x, y) in programs::sample_points() {
            let stack = reference::evaluate_shading(&witness, x, y);
            let slots = lowered::run_lowered(&analysis, &witness, x, y);
            assert_eq!(
                stack.map(f32::to_bits),
                slots.map(f32::to_bits),
                "{} at ({x}, {y}): stack {stack:?} against slots {slots:?}",
                witness.name
            );
        }
    }
}

/// The colour a program leaves after §7.10.1's output clip, at one point, through both
/// machines.
fn colour_at(program: &[FnOp], range: FnRange, x: f32, y: f32) -> [f32; 4] {
    let witness = programs::witness(program, range);
    let analysis = analyse(&witness.program).unwrap();
    let expected = reference::evaluate_shading(&witness, x, y);
    let lowered = lowered::run_lowered(&analysis, &witness, x, y);
    assert_eq!(expected, lowered, "the two machines disagree");
    expected
}

/// PLRM3's `round`: "returns the integer value nearest to num1. If num1 is equally close to
/// its two nearest integers, round returns the greater of the two", with `-6.5 round => -6.0`
/// as the entry's own example.
///
/// Neither built-in is this rule — Rust's `f32::round` is half-away-from-zero and gives −7,
/// WGSL's `round` is half-to-even and gives 2 for 2.5 where PLRM3 says 3. The four values
/// below separate all three rules.
#[test]
fn round_is_plrm3s_half_toward_greater() {
    let cases = [(-6.5_f32, -6.0_f32), (2.5, 3.0), (-2.5, -2.0), (-6.4, -6.0)];
    for (operand, expected) in cases {
        let colour = colour_at(
            &[FnOp::Pop, FnOp::Pop, FnOp::PushReal(operand), FnOp::Round],
            WIDE_GRAY,
            0.5,
            0.5,
        );
        assert_eq!(colour[0], expected, "{operand} round");
    }
    // And the rule this is *not*, stated so a reader can see the difference is deliberate.
    assert_eq!((-6.5_f32).round(), -7.0);
}

/// PLRM3's `exp`: `base exponent exp`, with the entry's own example `-9 -1 exp => -0.111111`.
/// WGSL's `pow` is "inherited from exp2(y * log2(x))" and undefined for a negative base, so
/// the operator cannot be `pow`.
#[test]
fn exp_admits_a_negative_base_with_an_integer_exponent() {
    let ninth = colour_at(
        &[
            FnOp::Pop,
            FnOp::Pop,
            FnOp::PushInt(-9),
            FnOp::PushInt(-1),
            FnOp::Exp,
        ],
        WIDE_GRAY,
        0.5,
        0.5,
    );
    assert!(
        (ninth[0] - -0.111_111).abs() < 1e-5,
        "-9 -1 exp is {}",
        ninth[0]
    );

    // An even exponent keeps the sign positive; a fractional one is the case PLRM3 calls
    // meaningful "only if the base is nonnegative", which is a guarded domain exit here.
    let squared = colour_at(
        &[
            FnOp::Pop,
            FnOp::Pop,
            FnOp::PushInt(-3),
            FnOp::PushInt(2),
            FnOp::Exp,
        ],
        WIDE_GRAY,
        0.5,
        0.5,
    );
    assert!(
        (squared[0] - 9.0).abs() < 1e-4,
        "-3 2 exp is {}",
        squared[0]
    );
}

/// PLRM3's `bitshift`: "Bits shifted out are lost; bits shifted in are 0", so a right shift is
/// a *logical* one. An `i32 >>` sign-extends and would shift ones in for a negative operand,
/// which is the reading this test exists to exclude.
#[test]
fn bitshift_shifts_zeros_in_from_both_ends() {
    let shifted = colour_at(
        &[
            FnOp::Pop,
            FnOp::Pop,
            FnOp::PushInt(-16),
            FnOp::PushInt(-28),
            FnOp::Bitshift,
        ],
        WIDE_GRAY,
        0.5,
        0.5,
    );
    // -16 is 0xFFFFFFF0; shifted right 28 places with zeros shifted in, that is 0xF.
    assert_eq!(shifted[0], 15.0);
    // The arithmetic reading would have produced -1; stated so the difference is visible.
    assert_ne!(shifted[0], -1.0);
}

/// Table 42's other `not`: PLRM3's one's complement over an integer, which makes `63 not`
/// equal `-64`. The caller's own evaluator implements the logical reading only and yields
/// `0.0` there; the value below comes from the clause, not from either implementation.
#[test]
fn not_over_an_integer_is_the_ones_complement() {
    let complemented = colour_at(programs::NOT_ON_INTEGER, WIDE_GRAY, 0.5, 0.5);
    assert_eq!(complemented[0], -64.0);
}

/// PLRM3's `atan` returns "the angle (in degrees between 0 and 360)", and `0 0 atan` is an
/// `undefinedresult` that has to be guarded rather than left to produce an indeterminate
/// value. Both values below follow from the definition rather than from an example.
#[test]
fn atan_is_in_degrees_over_the_whole_circle() {
    let quarter = colour_at(
        &[
            FnOp::Pop,
            FnOp::Pop,
            FnOp::PushInt(1),
            FnOp::PushInt(0),
            FnOp::Atan,
        ],
        WIDE_GRAY,
        0.5,
        0.5,
    );
    assert!(
        (quarter[0] - 90.0).abs() < 1e-3,
        "1 0 atan is {}",
        quarter[0]
    );

    let three_quarters = colour_at(
        &[
            FnOp::Pop,
            FnOp::Pop,
            FnOp::PushInt(-100),
            FnOp::PushInt(0),
            FnOp::Atan,
        ],
        WIDE_GRAY,
        0.5,
        0.5,
    );
    assert!(
        (three_quarters[0] - 270.0).abs() < 1e-3,
        "-100 0 atan is {}",
        three_quarters[0]
    );
}

/// ISO 32000-2 §8.7.4.5.2, verbatim:
///
/// > Points within the shading's bounding box (BBox) that fall outside this transformed domain
/// > rectangle shall be painted with the shading's background colour (Background); if the
/// > shading dictionary has no Background entry, such points shall be left unpainted.
///
/// **Discard, not clamp.** The domain here is a quarter of the area the points cover, which is
/// the shape neither of the caller's witnesses has: both declare `/Domain [0 1 0 1]`, so a
/// shader that clamped would agree with one that discards on every page they own.
#[test]
fn outside_the_domain_the_shading_paints_nothing() {
    let witness = programs::small_domain(None);
    let analysis = analyse(&witness.program).unwrap();

    // Inside: `x`, `y`, `x*y` at full coverage.
    let inside = lowered::run_lowered(&analysis, &witness, 0.25, 0.5);
    assert_eq!(inside, [0.25, 0.5, 0.125, 1.0]);

    // Outside, on each axis and on both: no colour and no coverage at all. The clamping
    // reading would have painted [0.5, 0.5, 0.25] at alpha 1 for the first of these.
    for (x, y) in [(0.75, 0.5), (0.25, 0.9), (2.0, 2.0), (-0.1, 0.25)] {
        let outside = lowered::run_lowered(&analysis, &witness, x, y);
        assert_eq!(outside, [0.0; 4], "({x}, {y}) should be unpainted");
        assert_eq!(reference::evaluate_shading(&witness, x, y), [0.0; 4]);
    }
}

/// The other half of the same clause: with a `Background`, the outside is that colour rather
/// than nothing — and it is emphatically not the edge colour a clamp would have produced.
#[test]
fn outside_the_domain_a_background_is_painted_instead() {
    let background = Color::new(0.1, 0.2, 0.3, 1.0);
    let witness = programs::small_domain(Some(background));
    let analysis = analyse(&witness.program).unwrap();

    let outside = lowered::run_lowered(&analysis, &witness, 0.75, 0.5);
    assert_eq!(outside, [0.1, 0.2, 0.3, 1.0]);
    // The clamped reading: x pinned to 0.5, y to 0.5, so [0.5, 0.5, 0.25]. Nothing here.
    assert_ne!(outside, [0.5, 0.5, 0.25, 1.0]);
}

/// ISO 32000-2 §7.10.1: "output values produced by the function shall be clipped to the
/// range".
///
/// The range here is not the unit interval on any component and not the same on any two,
/// because both of the caller's witnesses declare `/Range [0 1 0 1 0 1]` — so a defect in this
/// clamp is invisible on their corpus and would surface only on a document like the one this
/// test invents.
#[test]
fn the_output_is_clipped_to_a_range_that_is_not_the_unit_interval() {
    // The program leaves 5.0, -3.0 and 0.5 against a range of [0.2, 0.8], [-1, 1], [10, 20].
    let colour = colour_at(programs::OUT_OF_RANGE, programs::ODD_RGB, 0.5, 0.5);
    assert_eq!(colour, [0.8, -1.0, 10.0, 1.0]);

    // The same program under the unit range, so the clamp is visibly doing the work rather
    // than the program happening to land there.
    let unit = colour_at(programs::OUT_OF_RANGE, programs::UNIT_RGB, 0.5, 0.5);
    assert_eq!(unit, [1.0, 0.0, 0.5, 1.0]);
}

/// A pop from an empty operand stack yields 0, and the program that leans on it computes what
/// that decision implies rather than something else.
#[test]
fn the_empty_stack_zero_reaches_the_arithmetic() {
    // Three pops on a stack of two, then `1.0 add`: the addend is the zero the third pop
    // yielded, so the answer is 1.0 and not the coordinate.
    let colour = colour_at(programs::EMPTY_STACK, WIDE_GRAY, 0.375, 0.625);
    assert_eq!(colour[0], 1.0);
}

/// The two machines agree about a witness with a background, at every sample point, which is
/// the only place the two clauses meet.
#[test]
fn the_two_machines_agree_about_a_background() {
    let witnesses: [Witness; 2] = [
        programs::small_domain(None),
        programs::small_domain(Some(Color::new(1.0, 0.0, 0.0, 0.5))),
    ];
    for witness in witnesses {
        let analysis = analyse(&witness.program).unwrap();
        for (x, y) in programs::sample_points() {
            assert_eq!(
                reference::evaluate_shading(&witness, x, y),
                lowered::run_lowered(&analysis, &witness, x, y),
                "at ({x}, {y})"
            );
        }
    }
}
