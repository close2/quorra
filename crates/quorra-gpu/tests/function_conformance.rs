//! The ISO 32000-2 §7.10.5 conformance corpus, run **on the device** and judged by its own
//! reference evaluator.
//!
//! `quorra-function-conformance` holds 125 cases over all 42 of Table 42's operators, each
//! with the clause or PLRM3 entry its expectation came from, and an evaluator written
//! separately from the same documents. Until this file, nothing ran any of it on a GPU:
//! ADR 0053's last consequence — *"the classification needs a conformance test per dangerous
//! and per safe operator before it is a contract"* — is what this closes.
//!
//! # The three things asserted, and why they are three
//!
//! 1. **Every refusal ground the corpus names is actually refused**, and refused before a
//!    frame: at `Device::upload_function` where the ground is a property of the program, and
//!    at `Analysis::admits` where it is a property of the program *with* a `Range`. A ground
//!    nobody reaches is not a ground; a ground nobody *enforces* is worse.
//! 2. **Every case the device admits computes what the reference computes**, to a bound
//!    stated below and justified from WGSL's own accuracy table rather than from a run.
//! 3. **Nothing is skipped silently.** Every case lands in exactly one bucket, the buckets
//!    are counted, and the counts are asserted — a corpus run that quietly compared nothing
//!    would otherwise pass.
//!
//! # The bound, and where it comes from
//!
//! Two comparisons, chosen by a property of the program rather than by what passed:
//!
//! - **A program with no inexact operator in it is compared bit for bit.** That is stricter
//!   than `Agreement::Bounded`, deliberately: `Bounded` means no inexact operator's value
//!   reaches an *amplifier*, which still permits a last-bit difference in the colour, while
//!   a program that calls none of `atan`, `sin`, `cos`, `exp`, `ln`, `log`, `sqrt` or `div`
//!   has nothing WGSL §15.7.4.1 licenses a difference in.
//! - **Everything else is compared to 1e-3, relative or absolute, whichever is larger.**
//!   The loosest row of WGSL §15.7.4.1 is `atan` at **4 096 ULP**, which at a result of
//!   magnitude *m* is `m × 4 096 × 2⁻²³ ≈ m × 4.9e-4` — inside a relative 1e-3 at every
//!   magnitude, and inside an absolute 1e-3 below 1. `sin` and `cos` are stated as an
//!   absolute 2⁻¹¹ ≈ 4.9e-4 over `[-π, π]`; `div` is 2.5 ULP and `sqrt` is inherited from
//!   `inverseSqrt`, both far tighter. **The bound is this test's instrument and not a claim
//!   about ISO 32000-2**, which states no precision at all (§7.3.3, and ADR 0053 §2).
//!
//! Neither is a claim about a second adapter. ADR 0053's consequence stands: cross-adapter
//! identity is not promised for this paint, so every message names the adapter it ran on.
//!
//! # What this file deliberately does not run
//!
//! §7.10.1's **domain clip**. Its four cases pass an input outside the domain and expect the
//! clipped value, and a §8.7.4.5.2 type 1 shading never asks its function about such a
//! point: the clause discards it or paints the background instead. The clip is real and the
//! reference implements it; what cannot exist is a *shading* that observes it. The count of
//! those cases is asserted, so a fifth one appearing does not pass unnoticed.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::cast_precision_loss
)]

mod function_support;

use function_support::compute::{Compute, Shading};
use quorra_function_conformance::case::{Case, Expectation, Refusal, Subject};
use quorra_function_conformance::{cases, evaluate_case};
use quorra_gpu::error::FunctionProblem;
use quorra_gpu::function::FunctionRefusal;
use quorra_gpu::{Device, Options};
use quorra_scene::{FnOp, FnRange};

/// The relative-or-absolute bound for a program that calls an operator WGSL declines to
/// specify tightly. Its derivation is in this file's header; it is the test's instrument.
const INEXACT_BOUND: f32 = 1e-3;

/// The operators WGSL §15.7.4.1 gives a loose accuracy or none at all. A program free of all
/// of them is one the device and the host should agree with bit for bit.
fn calls_an_inexact_operator(program: &[FnOp]) -> bool {
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

/// A headless device on the software adapter, as everywhere in this crate's tests.
fn device() -> Device {
    let requested = std::env::var("QUORRA_ADAPTER").unwrap_or_else(|_| "llvmpipe".into());
    Device::headless(&Options {
        adapter: Some(requested),
        ..Options::default()
    })
    .expect("the requested adapter is present")
}

/// Where the device stands on one case's program: admitted, or refused with the reason.
enum Disposition {
    Admitted,
    Refused(FunctionProblem),
}

/// Upload the case's program as a two-input function, which is the shape a
/// `Paint::Function` has, and say what the device made of it.
fn upload(device: &mut Device, case: &Case) -> Disposition {
    match device.upload_function(&case.two_input_program()) {
        Ok(_) => Disposition::Admitted,
        Err(quorra_gpu::DeviceError::InvalidFunction { reason }) => Disposition::Refused(reason),
        Err(other) => panic!(
            "{}: an upload failed for an unrelated reason: {other}",
            case.name
        ),
    }
}

/// Whether the device refuses this case's program together with this case's `Range` — the
/// second half of "before the frame", for the grounds that are a property of the pairing
/// rather than of the program.
fn refused_with_its_range(case: &Case) -> Option<FunctionRefusal> {
    let program = case.two_input_program();
    let analysis = quorra_gpu::function::analyse(&program).ok()?;
    analysis.admits(case.range).err()
}

/// **Every ground the corpus names is refused before a frame**, and the ground it is refused
/// on is the one the corpus named.
///
/// The two places are not interchangeable: a program that cannot be lowered at all is
/// refused at the upload, where the caller can fall back before building a scene; a program
/// that cannot fill *this* `Range` is refused when the two meet, which is the earliest
/// moment the question exists.
#[test]
fn every_refusal_ground_is_refused_before_a_frame() {
    let mut device = device();
    let mut covered: Vec<Refusal> = Vec::new();

    for case in cases() {
        let Expectation::Refused(ground) = case.expect else {
            continue;
        };
        let refusal = match upload(&mut device, case) {
            Disposition::Refused(reason) => Some(reason),
            // Not a defect by itself: some grounds are about the program *with* a Range.
            Disposition::Admitted => refused_with_its_range(case).map(FunctionProblem::Program),
        };
        let refusal = refusal.unwrap_or_else(|| {
            panic!(
                "{}: the corpus names the ground {ground:?} and the device drew it",
                case.name
            )
        });
        assert!(
            names_the_ground(&refusal, ground),
            "{}: refused as {refusal}, which is not {ground:?}",
            case.name
        );
        if !covered.contains(&ground) {
            covered.push(ground);
        }
    }

    for ground in Refusal::ALL {
        assert!(
            covered.contains(&ground),
            "no case reached {ground:?} through the device's own refusal path"
        );
    }
}

/// Whether the refusal the device gave is the ground the corpus named.
///
/// The mapping is written out rather than matched on a string: the corpus's vocabulary is
/// the clause's and this crate's is the lowering's, and where they differ in *name* they
/// must not be allowed to differ in *meaning* by accident.
fn names_the_ground(refusal: &FunctionProblem, ground: Refusal) -> bool {
    matches!(
        (refusal, ground),
        (
            FunctionProblem::Program(FunctionRefusal::StackTooDeep { .. }),
            Refusal::OperandStackTooDeep,
        ) | (
            FunctionProblem::Structure(quorra_scene::SceneError::BackwardFunctionJump { .. }),
            Refusal::BackwardJump,
        ) | (
            FunctionProblem::Structure(quorra_scene::SceneError::FunctionJumpOutOfRange { .. }),
            Refusal::JumpOutOfRange,
        ) | (
            FunctionProblem::Program(FunctionRefusal::OutputCount { .. }),
            Refusal::OutputCountMismatch,
        ) | (
            FunctionProblem::Program(FunctionRefusal::DynamicStackCount { .. }),
            Refusal::NonLiteralStackCount,
        ) | (
            FunctionProblem::Program(FunctionRefusal::UnbalancedBranches { .. }),
            Refusal::JoinDepthMismatch,
        ) | (
            FunctionProblem::Program(FunctionRefusal::UndecidableOperandType { .. }),
            Refusal::AmbiguousOperandType,
        )
    )
}

/// **The corpus, evaluated on the device, against the evaluator written from the clause.**
///
/// One point per case — the case's own inputs, which is what its expectation is about — and
/// the comparison is device against reference, never device against a number a previous run
/// produced.
#[test]
fn the_device_computes_what_the_corpus_evaluator_computes() {
    let Some(compute) = Compute::new() else {
        eprintln!("no adapter available; the conformance corpus did not run on a device");
        return;
    };
    let adapter = &compute.adapter;
    let mut compared = 0usize;
    let mut compared_bitwise = 0usize;
    let mut refused: Vec<(&str, String)> = Vec::new();
    let mut no_value = 0usize;
    let mut domain_clip = 0usize;

    for case in cases() {
        if case.subject == Subject::DomainClip {
            // See the header: a type 1 shading never asks its function about a point outside
            // the domain, so the input clip is not observable through this paint.
            domain_clip += 1;
            continue;
        }
        let Expectation::Outputs { .. } = case.expect else {
            no_value += 1;
            continue;
        };
        let Ok(want) = evaluate_case(case) else {
            // The reference declines to supply a value — an error PLRM3 names, or a silence
            // it records. The device supplies its guard value there, which is a decision
            // ADR 0053 records and not something the corpus can judge.
            no_value += 1;
            continue;
        };

        let program = case.two_input_program();
        let analysis = match quorra_gpu::function::admit(&program) {
            Ok(analysis) => analysis,
            Err(reason) => {
                refused.push((case.name, reason.to_string()));
                continue;
            }
        };
        if let Err(reason) = analysis.admits(case.range) {
            refused.push((case.name, reason.to_string()));
            continue;
        }

        let point = point_of(case);
        assert!(
            inside(point, case.domain),
            "{}: the case's own inputs are outside its own domain",
            case.name
        );
        let got = compute.run(
            &Shading {
                program: &program,
                range: case.range,
                domain: case.domain,
                background: None,
            },
            &[point],
        );
        let got = got.first().copied().unwrap_or([0.0; 4]);
        let bitwise = !calls_an_inexact_operator(&program);
        if bitwise {
            compared_bitwise += 1;
        }
        for (component, want) in want.outputs.iter().enumerate() {
            let got = channel(got, case.range, component);
            if bitwise {
                assert_eq!(
                    want.to_bits(),
                    got.to_bits(),
                    "{} on {adapter}, component {component}: the reference says {want} and \
                     the device says {got}; no operator in this program is one WGSL declines \
                     to specify",
                    case.name
                );
            } else {
                let bound = INEXACT_BOUND * want.abs().max(1.0);
                assert!(
                    (want - got).abs() <= bound,
                    "{} on {adapter}, component {component}: the reference says {want}, the \
                     device says {got}, and the stated bound is {bound}",
                    case.name
                );
            }
        }
        compared += 1;
    }

    // Every case is in exactly one bucket, and the buckets are printed so a change in the
    // corpus shows up as a number rather than as a silence.
    eprintln!(
        "{adapter}: {compared} cases compared ({compared_bitwise} bitwise), \
         {} refused before the frame, {no_value} carrying no value, \
         {domain_clip} domain-clip cases a shading cannot observe",
        refused.len()
    );
    for (name, reason) in &refused {
        eprintln!("  refused: {name} — {reason}");
    }
    assert_eq!(
        domain_clip, 4,
        "the four domain-clip cases are the ones a type 1 shading cannot observe"
    );
    assert!(
        compared >= 60,
        "only {compared} cases reached the device; a corpus run that compares nothing passes \
         for the wrong reason"
    );
    assert!(
        compared_bitwise >= 40,
        "only {compared_bitwise} cases took the bitwise comparison"
    );
}

/// **Every refusal the device takes over the whole corpus is one of a stated set** — and
/// there is no case it declines for a reason nobody wrote down.
///
/// The complement of the two tests above: those say the named grounds *are* refused, this
/// says nothing else is refused by accident. It runs over every case rather than only the
/// refusal family, so a case the corpus expects a value from that our analyser declines
/// fails here with its reason printed.
#[test]
fn every_refusal_the_device_takes_is_one_of_the_stated_reasons() {
    let mut taken = 0usize;
    for case in cases() {
        if case.subject == Subject::DomainClip {
            continue;
        }
        let program = case.two_input_program();
        let Err(reason) = quorra_gpu::function::admit(&program) else {
            continue;
        };
        taken += 1;
        let expected = match &reason {
            // The refusal family's four static grounds, each demonstrated by a case that
            // reaches it (`every_refusal_ground_is_refused_before_a_frame` is where the
            // ground and the refusal are matched up).
            FunctionProblem::Program(
                // The refusal family's static grounds, each demonstrated by a case that
                // reaches it (`every_refusal_ground_is_refused_before_a_frame` is where the
                // ground and the refusal are matched up).
                FunctionRefusal::StackTooDeep { .. }
                | FunctionRefusal::DynamicStackCount { .. }
                | FunctionRefusal::UnbalancedBranches { .. }
                // A type-sensitive operator over an operand of the wrong or undecidable
                // type: PLRM3 raises `typecheck` for the same programs, which is what
                // several of these cases expect as their own outcome.
                | FunctionRefusal::OperandType { .. }
                | FunctionRefusal::UndecidableOperandType { .. }
                // PLRM3 makes a negative `copy` count and an `index` past the bottom of the
                // stack a `rangecheck`, and the count is a literal, so the error is static
                // and the refusal is §5's "discoverable before the frame".
                | FunctionRefusal::StackCountOutOfRange { .. },
            )
            | FunctionProblem::Structure(
                quorra_scene::SceneError::BackwardFunctionJump { .. }
                | quorra_scene::SceneError::FunctionJumpOutOfRange { .. },
            )
            // ADR 0053 §3: a transcendental whose value reaches an amplifier has no
            // agreement bound to state, so the program is refused rather than drawn.
            | FunctionProblem::NoAgreementBound { .. } => true,
            _ => false,
        };
        assert!(
            expected,
            "{}: refused as {reason}, which is not one of the stated reasons",
            case.name
        );
    }
    // Not vacuous, and counted so that it cannot become so: the refusal family alone
    // reaches five of these grounds at the upload, and a corpus whose refusals stopped
    // being refused would fail here rather than pass quietly.
    assert!(
        taken >= 5,
        "only {taken} refusals were taken over the whole corpus"
    );
}

/// The point the device evaluates a case at: its own inputs, with the domain's centre
/// standing in for an input it does not declare.
///
/// A case declaring fewer than two inputs has the surplus popped by
/// `Case::two_input_program`, so what stands in for them is never read — the centre is
/// chosen because it is inside the domain, which the shading's own discard requires.
fn point_of(case: &Case) -> (f32, f32) {
    let centre = f32::midpoint;
    (
        case.inputs
            .first()
            .copied()
            .unwrap_or_else(|| centre(case.domain[0], case.domain[1])),
        case.inputs
            .get(1)
            .copied()
            .unwrap_or_else(|| centre(case.domain[2], case.domain[3])),
    )
}

/// Whether a point is inside a `[x_min, x_max, y_min, y_max]` domain, which is what the
/// generated shader requires before it runs the program at all (§8.7.4.5.2).
fn inside(point: (f32, f32), domain: [f32; 4]) -> bool {
    point.0 >= domain[0] && point.0 <= domain[1] && point.1 >= domain[2] && point.1 <= domain[3]
}

/// The channel a `Range`'s component lands in.
///
/// `DeviceGray` replicates its one component into all three, per §8.7.4.5.2 with §8.6.4.2,
/// so a one-component case is read from the red channel and the shader's replication is
/// checked by `function_lane.rs` where a colour is what is being asserted.
fn channel(colour: [f32; 4], range: FnRange, component: usize) -> f32 {
    match range {
        FnRange::Gray(_) => colour[0],
        FnRange::Rgb(_) => colour.get(component).copied().unwrap_or(0.0),
    }
}
