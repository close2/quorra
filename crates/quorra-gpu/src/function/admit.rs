//! The three questions an upload asks, in the order that makes a refusal cheap.
//!
//! One responsibility: **decide whether a device will execute a program, before any frame
//! exists.** `Device::upload_function` is this function plus a place to keep what it
//! returns, and the split is deliberate — the decision needs no adapter, no queue and no
//! `wgpu`, so it is testable without one and it is stated where a reader can check it
//! against the clause rather than against a device.
//!
//! The three questions, and why they are three rather than one:
//!
//! 1. **Is the instruction list executable at all?** `quorra_scene::check_program` — a
//!    non-empty list, within the vocabulary's own length bound, every jump strictly
//!    forward and inside. It belongs to `quorra-scene` because it is about the vocabulary
//!    that crate defines, and a caller may want the answer with no device in hand.
//! 2. **Can a shader be generated from it?** [`analyse`] — the depth, the slot types, the
//!    resolved `copy`/`index`/`roll` counts, the branch structure. Everything a generated
//!    shader has to know statically, computed rather than trusted.
//! 3. **Can an independent evaluation of it be expected to agree with ours?**
//!    [`Analysis::agreement`], which is ADR 0053 §3's classification.
//!
//! # The third question is the one that is a decision rather than a check
//!
//! ADR 0053 §3, on a program whose value from a transcendental reaches a comparison:
//!
//! > a program that can reach a transcendental on any path into a comparison is refused by
//! > name, and the caller falls back to the raster they build today.
//!
//! We can *draw* such a program. What we cannot do is tell the caller what their own
//! evaluation of it would have to match, because our two adapters do not agree with each
//! other on `sin`, `cos`, `exp`, `sqrt`, `div` or `atan` (ADR 0053's table: 3 201 of 4 096
//! inputs differ on `sin` between RADV and llvmpipe), and a comparison downstream of one of
//! those turns a last-bit difference into a whole colour over a set of points the program
//! itself chooses. So the answer is a refusal at the upload, where the caller can still
//! fall back cheaply, rather than a `Report` on a page it has already drawn.
//!
//! The amendment to ADR 0053 renamed the classification but did not move this line: what
//! the accepted class promises is a *bounded colour difference*, which is the currency
//! §10.7.3 measures a shading's accuracy in, and what the refused class has is no bound at
//! all.

use quorra_scene::FnOp;

use super::analyse::{Analysis, analyse};
use super::lowered::Agreement;
use crate::error::FunctionProblem;

/// Admit a program for execution on a device, or name what stops it.
///
/// # Errors
///
/// [`FunctionProblem`], naming which of the three questions above was answered no and what
/// answered it.
pub fn admit(program: &[FnOp]) -> Result<Analysis, FunctionProblem> {
    quorra_scene::check_program(program).map_err(FunctionProblem::Structure)?;
    let analysis = analyse(program).map_err(FunctionProblem::Program)?;
    if let Agreement::Unbounded {
        inexact,
        inexact_at,
        amplifier,
        amplifier_at,
    } = analysis.agreement()
    {
        return Err(FunctionProblem::NoAgreementBound {
            inexact,
            inexact_at,
            amplifier,
            amplifier_at,
        });
    }
    Ok(analysis)
}

#[cfg(test)]
mod tests {
    use quorra_scene::FnOp;

    use super::admit;
    use crate::error::FunctionProblem;
    use crate::function::{Agreement, FunctionRefusal};

    /// The order of the three questions is observable, and it is the order that makes a
    /// refusal cheapest: an empty program is refused by the structural check without the
    /// walk ever running.
    #[test]
    fn an_empty_program_is_refused_by_the_structural_check() {
        assert!(matches!(
            admit(&[]),
            Err(FunctionProblem::Structure(
                quorra_scene::SceneError::EmptyFunctionProgram
            ))
        ));
    }

    /// The analyser's refusal reaches the upload boundary by name rather than as a
    /// summary: a caller reading a log has to be able to attribute it without reproducing
    /// it (§5 of the brief).
    #[test]
    fn the_analysers_refusal_arrives_by_name() {
        // `7.5 2 idiv` — PLRM3 requires two integers, and a truncating lowering would
        // silently compute a different function.
        let program = [
            FnOp::Pop,
            FnOp::Pop,
            FnOp::PushReal(7.5),
            FnOp::PushInt(2),
            FnOp::Idiv,
        ];
        assert!(matches!(
            admit(&program),
            Err(FunctionProblem::Program(FunctionRefusal::OperandType {
                operator: "idiv",
                ..
            }))
        ));
    }

    /// ADR 0053 §3's decision: a transcendental whose value reaches a comparison has no
    /// agreement bound to state, so the program is refused at the upload where the caller
    /// can still fall back.
    #[test]
    fn a_transcendental_reaching_a_comparison_is_refused_with_both_operators() {
        // `x sin 0 ge` — the ADR's own example, and the pair our two adapters disagree
        // about on 2 of 4 096 inputs before any oracle is involved.
        let program = [
            FnOp::Pop,
            FnOp::Sin,
            FnOp::PushReal(0.0),
            FnOp::Ge,
            FnOp::PushReal(1.0),
            FnOp::Exch,
            FnOp::Pop,
        ];
        match admit(&program) {
            Err(FunctionProblem::NoAgreementBound {
                inexact, amplifier, ..
            }) => {
                assert_eq!(inexact, "sin");
                assert_eq!(amplifier, "ge");
            }
            other => panic!("expected a named agreement refusal, got {other:?}"),
        }
    }

    /// The same operator with nothing downstream of it to amplify the difference is
    /// admitted, which is what makes the refusal a statement about the *program* rather
    /// than about a list of operators.
    #[test]
    fn the_same_transcendental_without_an_amplifier_is_admitted() {
        let program = [FnOp::Pop, FnOp::Sin];
        let analysis = admit(&program).expect("no comparison, so a bound exists");
        assert_eq!(analysis.agreement(), Agreement::Bounded);
    }
}
