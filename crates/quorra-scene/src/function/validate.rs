//! What a function paint may not contain, and it is refused loudly.
//!
//! `doc/RENDER_LIBRARY.md` §4.7 is the rule and this is its application to
//! [`FunctionPaint`]: every condition below is checked at the scene boundary, before any
//! device sees the paint, and each failure is a [`SceneError`] variant naming the number
//! that failed. Nothing is clamped, repaired, or left for a shader generator to discover
//! — §5 wants a limit discoverable *before* the frame as well as an `Err` at it, and this
//! is the cheaper of the two places to answer.
//!
//! It lives beside the type rather than in `scene::validate` because what it checks is
//! the *program's* well-formedness, which is this module's responsibility and not the
//! scene's; [`SceneBuilder`](crate::scene::SceneBuilder) calls in from there, in its own
//! boundary order, so "what a scene may not contain" still has one entry point.
//!
//! # What is deliberately **not** checked here
//!
//! Stack depth, the static classification of the operators two adapters do not agree on
//! (ADR 0053 §3), and the resolution of `copy`/`index`/`roll` counts are all device-side
//! analysis: they answer "can *this* device draw it", they need a walk that models the
//! stack, and a scene is device-independent by construction (ADR 0001). A program that
//! passes every check here may still be refused by `quorra-gpu`, and that is the intended
//! division rather than a gap.

use std::sync::Arc;

use super::{FnOp, FnRange, FunctionPaint};
use crate::error::SceneError;
use crate::geom::Affine;
use crate::scene::MAX_COORDINATE;

/// The most instructions a [`FunctionPaint::program`] may hold.
///
/// ISO 32000-2 bounds a type 4 function's length nowhere, so this is a deliberate choice
/// of ours with its cost written down rather than a number read out of a clause.
///
/// The anchor is `doc/spike-function-paint.md`: the largest type 4 program in either of
/// the caller's two witnesses is **482 instructions**, and the generated shader for it
/// took **6.3 ms** to compile with a cold driver cache. 8 192 is seventeen times that
/// witness; extrapolating the compile cost linearly — an extrapolation, not a
/// measurement — puts a program at this bound near **107 ms** of pipeline compilation,
/// which is eighteen times the whole 5.9 ms CPU-rasteriser frame principle 2 measures
/// against. So the bound is a **ceiling that keeps a refusal cheap**, not a target: a
/// program approaching it will be refused by a device budget long before it is drawn
/// (ADR 0053 §1 — the interpreter shape lost the device outright at 482 instructions
/// times 32 million fragments, and that is a refusal that must happen before the frame).
///
/// It also bounds the memory a scene can be made to hold from one paint:
/// 8 192 × 8 bytes is 64 KiB of program, and [`Scene::cost`](crate::scene::Scene::cost)
/// reports what a scene actually retains.
pub const MAX_PROGRAM_LENGTH: usize = 8_192;

impl FunctionPaint {
    /// Whether this paint is well-formed data, or which condition it broke.
    ///
    /// The conditions are checked in the order a reader meets them — the domain the
    /// program is evaluated over, the matrix that reaches it, the range its results are
    /// clipped into, then the program itself — and each is one function below.
    ///
    /// # Errors
    ///
    /// Refuses a non-finite, inverted or unboundedly large `domain`; a non-finite,
    /// unboundedly large or non-invertible `matrix`; a non-finite or inverted `range`
    /// pair; an empty program; a program longer than [`MAX_PROGRAM_LENGTH`]; and a jump
    /// whose target is out of range or is not strictly forward. Each error carries the
    /// value that failed and, where there is one, the limit it exceeded.
    pub fn check(&self) -> Result<(), SceneError> {
        check_domain(self.domain)?;
        check_matrix(self.matrix)?;
        check_range(self.range)?;
        check_program(&self.program)?;
        Ok(())
    }
}

/// §8.7.4.5.2's `Domain` rectangle, in the order §4.7 states a rectangle's conditions:
/// finite, ordered, and inside [`MAX_COORDINATE`].
///
/// An *empty* domain — `x_min` equal to `x_max` — is accepted, the same way an empty
/// [`Rect`](crate::geom::Rect) is. It is a degenerate rectangle, not a malformed one, and
/// what a degenerate rectangle covers is §10.7.4's question rather than this boundary's
/// (Table 77's `BBox` note makes the same point for a zero-height bounding box).
fn check_domain(domain: [f32; 4]) -> Result<(), SceneError> {
    if !domain.iter().all(|v| v.is_finite()) {
        return Err(SceneError::NonFiniteFunctionDomain(domain));
    }
    if domain[0] > domain[1] || domain[2] > domain[3] {
        return Err(SceneError::UnorderedFunctionDomain(domain));
    }
    let magnitude = domain.iter().fold(0.0_f32, |acc, v| acc.max(v.abs()));
    if magnitude > MAX_COORDINATE {
        return Err(SceneError::FunctionDomainTooLarge {
            domain,
            limit: MAX_COORDINATE,
        });
    }
    Ok(())
}

/// §8.7.4.5.2's `Matrix`, under the scene's transform conditions plus one of its own.
///
/// The extra condition is invertibility, and it is load-bearing rather than tidy: the
/// matrix maps the shading's own space *into* the scene, and a fragment shader has to go
/// the other way to know which point of the domain it is standing on. A singular matrix
/// collapses the domain onto a line or a point, so there is no such position to compute —
/// and [`Affine::invert`] returning `None` is what says so, with no identity fallback,
/// because a substituted identity is the plausible-looking wrong answer §4.7 forbids.
fn check_matrix(matrix: Affine) -> Result<(), SceneError> {
    if !matrix.is_finite() {
        return Err(SceneError::NonFiniteTransform(matrix));
    }
    if matrix.max_coefficient() > MAX_COORDINATE {
        return Err(SceneError::TransformTooLarge {
            transform: matrix,
            limit: MAX_COORDINATE,
        });
    }
    if matrix.invert().is_none() {
        return Err(SceneError::SingularFunctionMatrix(matrix));
    }
    Ok(())
}

/// §7.10.1's `Range`: a clip, so each pair is finite and ordered — and nothing more, for
/// the reason [`FnRange`] states.
fn check_range(range: FnRange) -> Result<(), SceneError> {
    for pair in range.pairs() {
        if !pair[0].is_finite() || !pair[1].is_finite() {
            return Err(SceneError::NonFiniteFunctionRange(range));
        }
        if pair[0] > pair[1] {
            return Err(SceneError::UnorderedFunctionRange(range));
        }
    }
    Ok(())
}

/// The program: bounded, non-empty, and jumping only forwards.
///
/// The length is checked before the walk, so an unchecked number never sizes anything —
/// and after it, every index fits `u32` by construction, which is why the jump errors can
/// name their position without a fallible conversion.
fn check_program(program: &Arc<[FnOp]>) -> Result<(), SceneError> {
    if program.is_empty() {
        // §7.10.1: a function is a transformation that produces output values. An empty
        // program produces none, and the empty-stack rule (ADR 0053) would turn that into
        // a plausible black instead of a refusal.
        return Err(SceneError::EmptyFunctionProgram);
    }
    if program.len() > MAX_PROGRAM_LENGTH {
        return Err(SceneError::FunctionProgramTooLong {
            length: program.len(),
            limit: MAX_PROGRAM_LENGTH,
        });
    }
    // The length is at most MAX_PROGRAM_LENGTH by the check above, so it fits u32; the
    // conversion is written fallibly anyway, and its error is the same bound, so the
    // claim is enforced rather than asserted in a comment.
    let length = u32::try_from(program.len()).map_err(|_| SceneError::FunctionProgramTooLong {
        length: program.len(),
        limit: MAX_PROGRAM_LENGTH,
    })?;
    // Counting alongside the instructions rather than indexing them: every position a
    // jump can name is then a `u32` by construction.
    for (at, op) in (0..length).zip(program.iter()) {
        if let FnOp::Jump { target } | FnOp::JumpUnless { target } = *op {
            check_jump(at, target, length)?;
        }
    }
    Ok(())
}

/// One jump, and the two conditions in the order that makes the message useful.
///
/// Forward-only is checked first because it is the property the whole design rests on: a
/// flat list whose jumps only ever move forward cannot loop, so the program's length is a
/// bound on its own execution and a fragment shader needs no loop to run it. A backward
/// or self jump is not slow, it is unbounded, and no device budget can be stated over it.
///
/// `target == length` is legitimate and means "stop": it is how a trailing `if` lowers,
/// the true arm being the tail of the program.
fn check_jump(at: u32, target: u32, length: u32) -> Result<(), SceneError> {
    if target <= at {
        return Err(SceneError::BackwardFunctionJump { at, target });
    }
    if target > length {
        return Err(SceneError::FunctionJumpOutOfRange { at, target, length });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MAX_PROGRAM_LENGTH, check_domain, check_matrix, check_program, check_range};
    use crate::error::SceneError;
    use crate::function::{FnOp, FnRange, FunctionPaint};
    use crate::geom::Affine;
    use std::sync::Arc;

    /// The smallest thing a §8.7.4.5.2 shading can say: a `DeviceGray` function over the
    /// unit square that is constant. Both of the caller's witnesses declare
    /// `/Domain [0 1 0 1]`, so this is their shape with the program removed.
    fn gray_paint(program: &[FnOp]) -> FunctionPaint {
        FunctionPaint {
            program: Arc::from(program),
            domain: [0.0, 1.0, 0.0, 1.0],
            matrix: Affine::IDENTITY,
            range: FnRange::Gray([0.0, 1.0]),
        }
    }

    /// The positive case, stated first so that every refusal below differs from it in
    /// exactly one way.
    #[test]
    fn a_well_formed_function_paint_is_accepted() {
        gray_paint(&[FnOp::PushReal(0.5)])
            .check()
            .expect("a constant grey over the unit square is well-formed");
    }

    /// A three-component range is the other admitted shape, and it is accepted with the
    /// forward jumps an `ifelse` lowers to.
    #[test]
    fn a_device_rgb_program_with_forward_jumps_is_accepted() {
        let paint = FunctionPaint {
            range: FnRange::Rgb([[0.0, 1.0], [0.0, 1.0], [0.0, 1.0]]),
            program: Arc::from(
                [
                    FnOp::PushBool(true),
                    FnOp::JumpUnless { target: 4 },
                    FnOp::PushReal(1.0),
                    FnOp::Jump { target: 5 },
                    FnOp::PushReal(0.0),
                    FnOp::Dup,
                    FnOp::Dup,
                ]
                .as_slice(),
            ),
            ..gray_paint(&[FnOp::PushReal(0.0)])
        };
        paint.check().expect("forward jumps are the whole contract");
    }

    /// §4.7: a domain is a rectangle, and a rectangle's three conditions are finite,
    /// ordered and bounded — each refused by the variant that names it.
    #[test]
    fn a_malformed_domain_is_refused_at_the_boundary() {
        assert!(matches!(
            check_domain([0.0, f32::NAN, 0.0, 1.0]),
            Err(SceneError::NonFiniteFunctionDomain(_))
        ));
        assert!(matches!(
            check_domain([0.0, f32::INFINITY, 0.0, 1.0]),
            Err(SceneError::NonFiniteFunctionDomain(_))
        ));
        assert!(matches!(
            check_domain([1.0, 0.0, 0.0, 1.0]),
            Err(SceneError::UnorderedFunctionDomain(_))
        ));
        assert!(matches!(
            check_domain([0.0, 1.0, 1.0, 0.0]),
            Err(SceneError::UnorderedFunctionDomain(_))
        ));
        assert!(matches!(
            check_domain([0.0, 2e9, 0.0, 1.0]),
            Err(SceneError::FunctionDomainTooLarge { .. })
        ));
    }

    /// An empty domain clips every input to one value, which is a constant colour and a
    /// legitimate thing to ask for — the same reading that makes an empty rectangle and a
    /// blank scene legitimate (§5).
    #[test]
    fn an_empty_domain_is_accepted_as_a_constant() {
        check_domain([0.5, 0.5, 0.5, 0.5]).expect("a degenerate domain is not a malformed one");
    }

    /// The matrix is a transform under the scene's own two conditions, plus the one this
    /// paint adds: the device inverts it per fragment, so a matrix with no inverse names
    /// no point of the domain to evaluate.
    #[test]
    fn a_matrix_the_device_cannot_invert_is_refused() {
        assert!(matches!(
            check_matrix(Affine {
                e: f32::NAN,
                ..Affine::IDENTITY
            }),
            Err(SceneError::NonFiniteTransform(_))
        ));
        assert!(matches!(
            check_matrix(Affine::translate(2e9, 0.0)),
            Err(SceneError::TransformTooLarge { .. })
        ));
        assert!(matches!(
            check_matrix(Affine::scale(0.0, 1.0)),
            Err(SceneError::SingularFunctionMatrix(_))
        ));
        // Not merely a zero scale: any linear part of zero determinant collapses the
        // domain onto a line.
        assert!(matches!(
            check_matrix(Affine {
                a: 2.0,
                b: 4.0,
                c: 1.0,
                d: 2.0,
                e: 0.0,
                f: 0.0
            }),
            Err(SceneError::SingularFunctionMatrix(_))
        ));
    }

    /// §7.10.1 clips outputs to the range, so each pair must be a clip: finite, and with
    /// its minimum first.
    #[test]
    fn a_malformed_range_pair_is_refused_at_the_boundary() {
        assert!(matches!(
            check_range(FnRange::Gray([0.0, f32::NAN])),
            Err(SceneError::NonFiniteFunctionRange(_))
        ));
        assert!(matches!(
            check_range(FnRange::Gray([1.0, 0.0])),
            Err(SceneError::UnorderedFunctionRange(_))
        ));
        // The third component is as much a component as the first: a check that stopped
        // at the first pair would pass this.
        assert!(matches!(
            check_range(FnRange::Rgb([[0.0, 1.0], [0.0, 1.0], [1.0, 0.0]])),
            Err(SceneError::UnorderedFunctionRange(_))
        ));
    }

    /// A range wider than the colour space is **not** refused: §8.7.4.5.2's Table 78
    /// adjusts an out-of-range component to the nearest valid value afterwards, so a
    /// conforming document may declare one and rely on that.
    #[test]
    fn a_range_outside_the_unit_interval_is_accepted() {
        check_range(FnRange::Gray([-2.0, 7.5])).expect("a range is a clip, not a colour claim");
    }

    /// An empty program leaves no output value. The empty-stack rule would turn that into
    /// a plausible black, which is the outcome §5 refuses by name.
    #[test]
    fn an_empty_program_is_refused_at_the_boundary() {
        assert!(matches!(
            gray_paint(&[]).check(),
            Err(SceneError::EmptyFunctionProgram)
        ));
    }

    /// The bound exists so that a refusal is cheap and a scene's memory is bounded; it
    /// names itself, per §5's "an `Err` that names what overflowed".
    #[test]
    fn a_program_past_the_stated_bound_is_refused_with_its_limit() {
        let too_long = vec![FnOp::Pop; MAX_PROGRAM_LENGTH + 1];
        assert!(matches!(
            check_program(&Arc::from(too_long.as_slice())),
            Err(SceneError::FunctionProgramTooLong {
                length,
                limit: MAX_PROGRAM_LENGTH,
            }) if length == MAX_PROGRAM_LENGTH + 1
        ));
        let at_the_bound = vec![FnOp::Pop; MAX_PROGRAM_LENGTH];
        check_program(&Arc::from(at_the_bound.as_slice())).expect("the bound itself is admitted");
    }

    /// The property the whole design rests on: a jump that does not move strictly forward
    /// makes the program's length no bound on its own execution.
    #[test]
    fn a_backward_jump_is_refused_at_the_boundary() {
        let jump_back = FnOp::Jump { target: 0 };
        let program = [FnOp::PushReal(0.0), FnOp::PushReal(0.0), jump_back];
        assert!(matches!(
            check_program(&Arc::from(program.as_slice())),
            Err(SceneError::BackwardFunctionJump { at: 2, target: 0 })
        ));
    }

    /// A jump to itself is the same defect with a zero-length loop, and is refused by the
    /// same condition rather than falling through it.
    #[test]
    fn a_self_jump_is_refused_at_the_boundary() {
        let program = [FnOp::PushBool(true), FnOp::JumpUnless { target: 1 }];
        assert!(matches!(
            check_program(&Arc::from(program.as_slice())),
            Err(SceneError::BackwardFunctionJump { at: 1, target: 1 })
        ));
    }

    /// Past the end is a different defect from backwards, and gets its own name and the
    /// length it exceeded.
    #[test]
    fn a_jump_past_the_end_of_the_program_is_refused() {
        let program = [FnOp::PushBool(true), FnOp::JumpUnless { target: 9 }];
        assert!(matches!(
            check_program(&Arc::from(program.as_slice())),
            Err(SceneError::FunctionJumpOutOfRange {
                at: 1,
                target: 9,
                length: 2
            })
        ));
    }

    /// A jump *to* the end is how a trailing `if` lowers — the true arm is the tail — so
    /// it is accepted, and it means "stop".
    #[test]
    fn a_jump_to_the_end_of_the_program_terminates_it() {
        let program = [
            FnOp::PushBool(true),
            FnOp::JumpUnless { target: 3 },
            FnOp::PushReal(1.0),
        ];
        check_program(&Arc::from(program.as_slice()))
            .expect("falling off the end is how a trailing conditional finishes");
    }

    /// Order matters at the boundary as much as the conditions do: a paint broken in two
    /// ways reports the one a reader meets first, so the message is about the outermost
    /// defect rather than whichever check happened to run.
    #[test]
    fn the_first_condition_a_reader_meets_is_the_one_reported() {
        let paint = FunctionPaint {
            domain: [1.0, 0.0, 0.0, 1.0],
            matrix: Affine::scale(0.0, 0.0),
            range: FnRange::Gray([1.0, 0.0]),
            program: Arc::from([].as_slice()),
        };
        assert!(matches!(
            paint.check(),
            Err(SceneError::UnorderedFunctionDomain(_))
        ));
    }
}
