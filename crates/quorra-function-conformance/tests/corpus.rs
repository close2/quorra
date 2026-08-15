//! The gate: every case in the corpus, through the reference evaluator, against its own
//! expectation.
//!
//! This is the test `doc/adr/0053` asked for before the classification could be a
//! contract. It is deliberately not a device test — there is no device here — and it is
//! not a test of the evaluator either. It is a test of **agreement between two readings
//! of the same clause**: the expectation was written from PLRM3's operator entry and the
//! evaluator was written from the same entry, separately, and a disagreement means one
//! of them misread it. Finding out which is the point; `doc/notes-function-conformance.md`
//! records what the corpus found on the way in.

// Test-file lint policy as in the rest of this workspace's suites: a test panics on
// purpose, and its arithmetic indexes bounded, literal programs. `float_cmp` is allowed
// because `Tolerance::Exact` means exactly that — an expectation the clause fixes to an
// integer or a rounding, where a margin would be the corpus quietly not checking it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::float_cmp
)]

use quorra_function_conformance::case::{Expectation, Refusal, Subject, Tolerance};
use quorra_function_conformance::reference::error::EvalError;
use quorra_function_conformance::table42::Table42;
use quorra_function_conformance::{Case, Evaluation, corpus, evaluate_case};

/// Every expectation, checked. A failure lists every case that disagreed rather than
/// the first, because a change of reading usually moves a family at a time.
#[test]
fn every_expectation_agrees_with_the_reference_evaluator() {
    let mut failures = Vec::new();
    for family in corpus::FAMILIES {
        for case in family.cases {
            if let Err(reason) = check(case) {
                failures.push(format!("{}/{}: {reason}", family.name, case.name));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases disagree with the reference evaluator:\n  {}",
        failures.len(),
        corpus::cases().count(),
        failures.join("\n  ")
    );
}

fn check(case: &Case) -> Result<(), String> {
    let outcome = evaluate_case(case);
    match case.expect {
        Expectation::Outputs {
            values,
            tolerance,
            report,
        } => {
            let evaluation = outcome.map_err(|error| format!("expected outputs, got {error}"))?;
            check_outputs(case, &evaluation, values, tolerance)?;
            check_reports(&evaluation, report)
        }
        Expectation::Error(expected) => match outcome {
            Err(EvalError::Error(actual)) if actual == expected => Ok(()),
            other => Err(format!("expected {expected:?}, got {other:?}")),
        },
        Expectation::Undefined { .. } => match outcome {
            Err(EvalError::Undefined(_)) => Ok(()),
            other => Err(format!(
                "expected the evaluator to decline to invent a value, got {other:?}"
            )),
        },
        // A ground the evaluator can reach must make it refuse; a ground only a static
        // walk can see must *not*, or the corpus has mislabelled which kind it is.
        Expectation::Refused(ground) => match (ground.reached_by_evaluation(), outcome) {
            (true, Err(_)) | (false, Ok(_)) => Ok(()),
            (true, Ok(evaluation)) => Err(format!(
                "{ground:?} is marked as reachable by evaluation, but the program \
                 evaluated to {:?}",
                evaluation.outputs
            )),
            (false, Err(error)) => Err(format!(
                "{ground:?} is marked as visible only to a static walk, but the \
                 evaluator refused it: {error}"
            )),
        },
    }
}

fn check_outputs(
    case: &Case,
    evaluation: &Evaluation,
    values: &[f32],
    tolerance: Tolerance,
) -> Result<(), String> {
    if values.len() != case.range.components() {
        return Err(format!(
            "the case expects {} values but its Range has {} components",
            values.len(),
            case.range.components()
        ));
    }
    if evaluation.outputs.len() != values.len() {
        return Err(format!(
            "expected {values:?}, got {:?}",
            evaluation.outputs.as_slice()
        ));
    }
    for (index, (&expected, &actual)) in values.iter().zip(evaluation.outputs.iter()).enumerate() {
        let within = match tolerance {
            Tolerance::Exact => actual == expected,
            Tolerance::Absolute(bound) => (actual - expected).abs() <= bound,
        };
        if !within {
            return Err(format!(
                "output {index}: expected {expected} ({tolerance:?}), got {actual}"
            ));
        }
    }
    Ok(())
}

fn check_reports(
    evaluation: &Evaluation,
    expected: Option<quorra_function_conformance::Report>,
) -> Result<(), String> {
    let actual = evaluation.reports.as_slice();
    match expected {
        Some(report) if actual == [report] => Ok(()),
        None if actual.is_empty() => Ok(()),
        _ => Err(format!("expected reports {expected:?}, got {actual:?}")),
    }
}

/// Every one of Table 42's 42 operators has at least one case.
#[test]
fn the_corpus_covers_every_table_42_operator() {
    let missing: Vec<&str> = Table42::ALL
        .iter()
        .filter(|operator| {
            !corpus::cases().any(|case| case.subject == Subject::Operator(**operator))
        })
        .map(|operator| operator.name())
        .collect();
    assert!(
        missing.is_empty(),
        "Table 42 operators with no case: {missing:?}"
    );
}

/// Every refusal ground has a program that reaches it — "a ground nobody can reach is
/// not a ground".
#[test]
fn the_corpus_covers_every_refusal_ground() {
    let missing: Vec<Refusal> = Refusal::ALL
        .iter()
        .filter(|ground| !corpus::cases().any(|case| case.expect == Expectation::Refused(**ground)))
        .copied()
        .collect();
    assert!(
        missing.is_empty(),
        "refusal grounds with no case: {missing:?}"
    );
}

/// The three concerns that belong to no operator each have cases, because those are the
/// ones an operator-by-operator corpus loses.
#[test]
fn the_corpus_covers_the_rules_that_belong_to_no_operator() {
    for subject in [
        Subject::DomainClip,
        Subject::RangeClip,
        Subject::OutputCount,
        Subject::EmptyStackPop,
    ] {
        assert!(
            corpus::cases().any(|case| case.subject == subject),
            "no case for {subject:?}"
        );
    }
}

/// A case's name is how a failure is reported, so two cases may not share one.
#[test]
fn case_names_are_unique() {
    let mut names: Vec<&str> = corpus::cases().map(|case| case.name).collect();
    let total = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(total, names.len(), "duplicate case names in the corpus");
}

/// Every expectation says where it came from. The check is shallow on purpose — a test
/// cannot read a citation — but an empty one, or one that names no source, is a case
/// somebody wrote without opening the document.
#[test]
fn every_case_cites_a_source() {
    for case in corpus::cases() {
        assert!(
            case.citation.contains("ISO 32000-2")
                || case.citation.contains("PLRM3")
                || case.citation.contains("§"),
            "{}: citation names no source: {:?}",
            case.name,
            case.citation
        );
    }
}

/// Adapting a case to the two-input shape a `Paint::Function` has moves every jump target
/// by the number of instructions prepended. This is the part that is easy to get wrong
/// and silent when it is: an off-by-two target still runs, and draws something.
#[test]
fn the_two_input_adaptor_moves_jump_targets() {
    for case in corpus::cases() {
        let adapted = case.two_input_program();
        let prefix = 2usize.saturating_sub(case.inputs.len());
        assert_eq!(
            adapted.len(),
            case.program.len() + prefix,
            "{}: wrong length after adaptation",
            case.name
        );
        for (original, moved) in case.program.iter().zip(adapted.iter().skip(prefix)) {
            let shift = |op: &quorra_scene::function::FnOp| match *op {
                quorra_scene::function::FnOp::Jump { target }
                | quorra_scene::function::FnOp::JumpUnless { target } => Some(target),
                _ => None,
            };
            match (shift(original), shift(moved)) {
                (Some(before), Some(after)) => assert_eq!(
                    after as usize,
                    before as usize + prefix,
                    "{}: jump target not moved",
                    case.name
                ),
                (None, None) => {}
                _ => panic!("{}: adaptation changed an instruction's kind", case.name),
            }
        }
    }
}

/// The adapted program is a legal two-input function: running it with the case's own
/// inputs, plus whatever fills the surplus, produces the case's own outputs.
#[test]
fn the_two_input_adaptor_preserves_what_a_case_computes() {
    for case in corpus::cases() {
        let Expectation::Outputs { values, .. } = case.expect else {
            continue;
        };
        let adapted = case.two_input_program();
        let mut inputs = case.inputs.to_vec();
        // The surplus is discarded by the prepended `Pop`s, so its value cannot matter;
        // a value the case never uses is the right thing to push here.
        while inputs.len() < 2 {
            inputs.push(0.75);
        }
        let evaluation =
            quorra_function_conformance::evaluate(&adapted, &inputs, case.domain, case.range);
        match evaluation {
            Ok(evaluation) => assert_eq!(
                evaluation.outputs.len(),
                values.len(),
                "{}: adapted program left the wrong number of outputs",
                case.name
            ),
            Err(error) => panic!("{}: adapted program failed: {error}", case.name),
        }
    }
}
