//! The generated WGSL, compiled and run, against an independent host evaluation of the same
//! program.
//!
//! This is the test that checks the *arithmetic* — `function_lowering.rs` checks the slot
//! allocation with the arithmetic held constant, and neither test alone would find a wrong
//! `ps_round`.
//!
//! # Why a compute pass over a buffer rather than a raster
//!
//! `doc/spike-function-paint.md` §5 measured 246 044 texels off by one between RADV and an
//! independent evaluation, and none of them were the program: they were ADR 0006's 8-bit
//! store conversion, one step, on one adapter. A raster puts that conversion between the
//! shader and the assertion and costs the test all of its resolution. Writing `vec4<f32>` to
//! a storage buffer removes it, so a difference this test sees is a difference the *program*
//! produced.
//!
//! # What a failure here means, and what it does not
//!
//! Bitwise equality is asserted only for programs with no inexact operator in them. For the
//! rest the tolerance is explicit and generous, because WGSL §15.7.4.1 licenses the
//! disagreement: 2.5 ULP on `div`, 4 096 ULP on `atan`, an absolute 2⁻¹¹ on `sin`. A tighter
//! bound would be a promise about a driver rather than a check on this crate.
//!
//! The test is skipped, loudly, where no adapter is available.

#![allow(
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects,
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "byte offsets and dispatch counts over a sample grid whose size this file fixes; \
              every expected value is an exact number a clause states, so an epsilon would \
              weaken the assertion into one three different rounding rules pass; and the \
              helpers are scaffolding whose panic is the test failing"
)]

mod function_support;

use function_support::compute::{Compute, Shading};
use function_support::programs::{self, Witness};
use function_support::reference;
use quorra_gpu::function::{Agreement, analyse, domain_bounds};
use quorra_scene::Color;

/// One program on the device at every sample point, through the shared compute harness.
///
/// The harness is `function_support::compute` rather than this file's own, because
/// `function_conformance.rs` runs the same shape over the conformance corpus and two copies
/// of a device harness are two things to keep in step.
fn run(compute: &Compute, witness: &Witness, points: &[(f32, f32)]) -> Vec<[f32; 4]> {
    compute.run(
        &Shading {
            program: &witness.program,
            range: witness.range,
            domain: domain_bounds(witness.domain),
            background: witness.background,
        },
        points,
    )
}

/// Every witness, on whatever adapter is here, against the host evaluation.
///
/// The two assertions differ by classification, which is the point: `Agreement::Bounded` means
/// no inexact operator's value reaches an amplifier, and for a program with *no* inexact
/// operator at all the two sides should land on the same bits.
#[test]
fn the_device_computes_what_the_host_computes() {
    let Some(compute) = Compute::new() else {
        eprintln!("no adapter available; the device half of the function tests did not run");
        return;
    };
    let adapter = &compute.adapter;
    let points = programs::sample_points();
    let mut checked = 0usize;
    // Counted, not assumed: a test whose strict branch never fires is a test that passed
    // because it asked an easier question, and nothing in the assertions above would say so.
    let mut compared_bitwise = 0usize;

    for witness in programs::all() {
        let analysis = analyse(&witness.program).unwrap();
        let colours = run(&compute, &witness, &points);
        assert_eq!(colours.len(), points.len(), "{}", witness.name);

        let bitwise = analysis.agreement() == Agreement::Bounded
            && !uses_an_inexact_operator(&witness.program);
        if bitwise {
            compared_bitwise += 1;
        }
        for ((x, y), got) in points.iter().copied().zip(&colours) {
            let want = reference::evaluate_shading(&witness, x, y);
            for channel in 0..4 {
                let (want, got) = (want[channel], got[channel]);
                if bitwise {
                    assert_eq!(
                        want.to_bits(),
                        got.to_bits(),
                        "{} on {adapter} at ({x}, {y}), channel {channel}: {want} against {got}",
                        witness.name
                    );
                } else {
                    let tolerance = 1e-4 * want.abs().max(1.0);
                    assert!(
                        (want - got).abs() <= tolerance,
                        "{} on {adapter} at ({x}, {y}), channel {channel}: {want} against {got}",
                        witness.name
                    );
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 0);
    assert!(
        compared_bitwise >= 8,
        "only {compared_bitwise} witnesses took the bitwise path on {adapter}"
    );
}

/// The operators WGSL declines to specify as tightly as IEEE 754 does. A program free of all
/// of them is one the two sides should agree with bit for bit.
fn uses_an_inexact_operator(program: &[quorra_scene::FnOp]) -> bool {
    use quorra_scene::FnOp;
    program.iter().any(|op| {
        matches!(
            op,
            FnOp::Atan
                | FnOp::Cos
                | FnOp::Div
                | FnOp::Exp
                | FnOp::Ln
                | FnOp::Log
                | FnOp::Sin
                | FnOp::Sqrt
        )
    })
}

/// ISO 32000-2 §8.7.4.5.2's discard, on the device rather than only in Rust.
///
/// The domain here is a quarter of the area the points cover — the shape neither of the
/// caller's witnesses has, and the one where a shader that clamped would differ from one that
/// discards. Both halves of the clause are checked: no background leaves the outside at zero
/// coverage, and a background paints exactly itself there.
#[test]
fn the_domain_rule_survives_the_shader() {
    let Some(compute) = Compute::new() else {
        eprintln!("no adapter available; the device half of the function tests did not run");
        return;
    };
    let adapter = &compute.adapter;
    let inside = (0.25_f32, 0.5_f32);
    let outside = (0.75_f32, 0.5_f32);
    // What a shader that clamped into the domain would have produced at `outside`.
    let clamped = [0.5_f32, 0.5, 0.25, 1.0];

    let unpainted = programs::small_domain(None);
    let got = run(&compute, &unpainted, &[inside, outside]);
    assert_eq!(
        got.first().copied(),
        Some([0.25, 0.5, 0.125, 1.0]),
        "{adapter}"
    );
    assert_eq!(got.get(1).copied(), Some([0.0; 4]), "{adapter}");
    assert_ne!(got.get(1).copied(), Some(clamped));

    let background = Color::new(0.1, 0.2, 0.3, 1.0);
    let painted = programs::small_domain(Some(background));
    let got = run(&compute, &painted, &[inside, outside]);
    assert_eq!(got.get(1).copied(), Some([0.1, 0.2, 0.3, 1.0]), "{adapter}");
    assert_ne!(got.get(1).copied(), Some(clamped));
}

/// The three operators whose value the specification states and whose built-ins do not
/// supply it, checked on the device rather than only in Rust — a `ps_round` written correctly
/// and compiled wrongly would pass every host test in the suite.
#[test]
fn the_specified_values_survive_the_shader() {
    use quorra_scene::FnOp;

    let Some(compute) = Compute::new() else {
        eprintln!("no adapter available; the device half of the function tests did not run");
        return;
    };
    let adapter = &compute.adapter;

    let cases: [(&'static str, Vec<FnOp>, f32); 4] = [
        // PLRM3: a tie goes to the greater of the two, so `-6.5 round` is `-6.0`. WGSL's own
        // `round` is half-to-even and Rust's is half-away-from-zero.
        (
            "-6.5 round",
            vec![FnOp::Pop, FnOp::Pop, FnOp::PushReal(-6.5), FnOp::Round],
            -6.0,
        ),
        // The same rule at a positive tie, where half-to-even would give 2.
        (
            "2.5 round",
            vec![FnOp::Pop, FnOp::Pop, FnOp::PushReal(2.5), FnOp::Round],
            3.0,
        ),
        // "Bits shifted out are lost; bits shifted in are 0" — a logical right shift, where
        // an `i32 >>` would sign-extend to -1.
        (
            "-16 -28 bitshift",
            vec![
                FnOp::Pop,
                FnOp::Pop,
                FnOp::PushInt(-16),
                FnOp::PushInt(-28),
                FnOp::Bitshift,
            ],
            15.0,
        ),
        // Table 42's other `not`: one's complement over an integer.
        ("63 not", programs::NOT_ON_INTEGER.to_vec(), -64.0),
    ];

    for (name, program, expected) in cases {
        let witness = programs::witness(&program, programs::WIDE_GRAY);
        let colours = run(&compute, &witness, &[(0.5, 0.5)]);
        assert_eq!(
            colours.first().map(|colour| colour[0]),
            Some(expected),
            "{name} on {adapter}"
        );
    }
}
