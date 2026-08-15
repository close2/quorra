//! What a [`Paint::Function`](crate::paint::Paint::Function) may not be, and the four
//! clauses behind it.
//!
//! ISO 32000-2 §8.7.4.5.2's type 1 shading is a program plus four numbers, and only the
//! numbers are the scene's business: the program is an uploaded resource, so whether it
//! can be executed was settled at `Device::upload_function` before this scene existed
//! (its structural half is [`check_program`](crate::function::check_program)).
//!
//! What is left is a rectangle, a matrix, a range and a colour — one §4.7 refusal each,
//! stated in the order §8.7.4.5.2 states its entries.

use super::SceneBuilder;
use crate::error::SceneError;
use crate::function::FnRange;
use crate::geom::{Affine, Rect};
use crate::paint::Color;

/// A [`Paint::Function`](crate::paint::Paint::Function)'s four numbers.
///
/// A free function rather than a method because
/// [`Paint::is_valid`](crate::paint::Paint::is_valid) asks the same question
/// without a builder in hand, and the rule is worth exactly one place to live.
///
/// The domain reuses the scene's own rectangle rule: a domain *is* a rectangle in the
/// shading's own space, and §4.7 says the same thing about it that it says about every
/// other rectangle. It is refused by [`SceneError::NonFiniteRect`],
/// [`SceneError::UnorderedRect`] or [`SceneError::RectTooLarge`] carrying the rectangle
/// that failed — a second set of names for one rule would make "how often does this
/// happen?" harder to answer, not easier. The matrix is the same argument again.
///
/// # Errors
///
/// Refuses a non-finite, inverted or oversized `domain`; a non-finite, oversized or
/// **singular** `matrix`; a `range` pair that is non-finite or inverted; and a background
/// colour outside `0..=1`.
pub(crate) fn check_function_paint(
    domain: Rect,
    matrix: Affine,
    range: FnRange,
    background: Option<Color>,
) -> Result<(), SceneError> {
    SceneBuilder::check_rect(domain)?;
    SceneBuilder::check_transform(matrix)?;
    // Table 78's `Matrix` maps *the domain into* the target space; a fragment shader has
    // to go the other way to know which point of the domain it is standing on. A singular
    // matrix collapses the domain onto a line, so there is no such point — and there is no
    // identity fallback, because a substituted identity is §4.7's plausible wrong answer.
    if matrix.invert().is_none() {
        return Err(SceneError::SingularFunctionMatrix(matrix));
    }
    check_range(range)?;
    // §8.7.4.5.2's `Background` arrives resolved to a device colour, so it is a colour
    // like any other; `None` is the clause's "left unpainted" and is not a defect.
    if let Some(background) = background {
        SceneBuilder::check_color(background)?;
    }
    Ok(())
}

/// §7.10.1's `Range`: a clip, so each pair is finite and ordered — and nothing more, for
/// the reason [`FnRange`] states.
fn check_range(range: FnRange) -> Result<(), SceneError> {
    for pair in range.bounds() {
        if !pair[0].is_finite() || !pair[1].is_finite() {
            return Err(SceneError::NonFiniteFunctionRange(range));
        }
        if pair[0] > pair[1] {
            return Err(SceneError::UnorderedFunctionRange(range));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check_function_paint;
    use crate::blend::{BlendMode, Compose, FillRule};
    use crate::error::SceneError;
    use crate::function::FnRange;
    use crate::geom::{Affine, Point, Rect};
    use crate::ids::{FunctionId, OutlineId};
    use crate::paint::{Color, Paint};
    use crate::scene::SceneBuilder;
    use crate::scene::fixtures::{function_paint, unit_rect};

    /// The entries both of the caller's witnesses declare, so every refusal below differs
    /// from a real document in exactly one way.
    fn entries() -> (Rect, Affine, FnRange, Option<Color>) {
        (
            unit_rect(),
            Affine::IDENTITY,
            FnRange::Gray([0.0, 1.0]),
            None,
        )
    }

    /// The positive case, and the one that says `None` is not a defect: a shading with no
    /// `/Background` leaves the outside of its domain unpainted, which is what
    /// §8.7.4.5.2 requires and not something to refuse.
    #[test]
    fn a_well_formed_function_paint_is_accepted() {
        let (domain, matrix, range, background) = entries();
        check_function_paint(domain, matrix, range, background)
            .expect("a DeviceGray shading over the unit square is well-formed");
        check_function_paint(domain, matrix, FnRange::Rgb([[0.0, 1.0]; 3]), background)
            .expect("three components are the other admitted shape");
        check_function_paint(domain, matrix, range, Some(Color::new(1.0, 0.0, 0.0, 1.0)))
            .expect("a background is a colour like any other");
    }

    /// A domain is a rectangle, so §4.7's rectangle rule is the rule — and the error names
    /// the rectangle that failed rather than inventing a second vocabulary for it.
    #[test]
    fn a_malformed_domain_is_refused_by_the_scene_rectangle_rule() {
        let (_, matrix, range, background) = entries();
        let check = |domain| check_function_paint(domain, matrix, range, background);
        assert!(matches!(
            check(Rect::new(Point::new(f32::NAN, 0.0), Point::new(1.0, 1.0))),
            Err(SceneError::NonFiniteRect(_))
        ));
        assert!(matches!(
            check(Rect::new(Point::new(5.0, 0.0), Point::new(1.0, 1.0))),
            Err(SceneError::UnorderedRect(_))
        ));
        assert!(matches!(
            check(Rect::new(Point::new(0.0, 0.0), Point::new(2e9, 1.0))),
            Err(SceneError::RectTooLarge { .. })
        ));
    }

    /// A degenerate domain rectangle is not a malformed one: what a zero-width rectangle
    /// covers is §10.7.4's question, and Table 77's `BBox` note makes the same point.
    #[test]
    fn a_degenerate_domain_is_accepted() {
        let (_, matrix, range, background) = entries();
        let flat = Rect::new(Point::new(0.5, 0.0), Point::new(0.5, 1.0));
        check_function_paint(flat, matrix, range, background)
            .expect("a degenerate rectangle is not a malformed one");
    }

    /// The matrix is a transform under the scene's own two conditions, plus the one this
    /// paint adds: the device inverts it per fragment, so a matrix with no inverse names
    /// no point of the domain to evaluate.
    #[test]
    fn a_matrix_the_device_cannot_invert_is_refused() {
        let (domain, _, range, background) = entries();
        let check = |matrix| check_function_paint(domain, matrix, range, background);
        assert!(matches!(
            check(Affine {
                e: f32::NAN,
                ..Affine::IDENTITY
            }),
            Err(SceneError::NonFiniteTransform(_))
        ));
        assert!(matches!(
            check(Affine::translate(2e9, 0.0)),
            Err(SceneError::TransformTooLarge { .. })
        ));
        assert!(matches!(
            check(Affine::scale(0.0, 1.0)),
            Err(SceneError::SingularFunctionMatrix(_))
        ));
        // Not merely a zero scale: any linear part of zero determinant collapses the
        // domain onto a line.
        assert!(matches!(
            check(Affine {
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
    /// its minimum first. An inverted pair returns the upper bound for every input.
    #[test]
    fn a_malformed_range_pair_is_refused_at_the_boundary() {
        let (domain, matrix, _, background) = entries();
        let check = |range| check_function_paint(domain, matrix, range, background);
        assert!(matches!(
            check(FnRange::Gray([0.0, f32::NAN])),
            Err(SceneError::NonFiniteFunctionRange(_))
        ));
        assert!(matches!(
            check(FnRange::Gray([1.0, 0.0])),
            Err(SceneError::UnorderedFunctionRange(_))
        ));
        // The third component is as much a component as the first: a check that stopped
        // at the first pair would pass this.
        assert!(matches!(
            check(FnRange::Rgb([[0.0, 1.0], [0.0, 1.0], [1.0, 0.0]])),
            Err(SceneError::UnorderedFunctionRange(_))
        ));
    }

    /// A range wider than the colour space is **not** refused: §8.7.4.5.2's Table 78
    /// adjusts an out-of-range component to the nearest valid value afterwards, so a
    /// conforming document may declare one and rely on that.
    #[test]
    fn a_range_outside_the_unit_interval_is_accepted() {
        let (domain, matrix, _, background) = entries();
        check_function_paint(domain, matrix, FnRange::Gray([-2.0, 7.5]), background)
            .expect("a range is a clip, not a colour claim");
    }

    /// A background is a colour, and a colour outside `0..=1` is refused by the name the
    /// scene already has for that.
    #[test]
    fn a_background_outside_the_unit_range_is_refused() {
        let (domain, matrix, range, _) = entries();
        assert!(matches!(
            check_function_paint(domain, matrix, range, Some(Color::new(0.0, 0.0, 0.0, 1.5))),
            Err(SceneError::InvalidColor(_))
        ));
    }

    /// Order matters at the boundary as much as the conditions do: a paint broken in
    /// three ways reports the one a reader meets first, so the message is about the
    /// outermost defect rather than whichever check happened to run.
    #[test]
    fn the_first_condition_a_reader_meets_is_the_one_reported() {
        assert!(matches!(
            check_function_paint(
                Rect::new(Point::new(1.0, 0.0), Point::new(0.0, 1.0)),
                Affine::scale(0.0, 0.0),
                FnRange::Gray([1.0, 0.0]),
                Some(Color::new(0.0, 0.0, 0.0, 9.0)),
            ),
            Err(SceneError::UnorderedRect(_))
        ));
    }

    /// The boundary asks: a fill takes a well-formed function paint and appends it, a
    /// stroke with a broken one is refused by name and appends nothing.
    #[test]
    fn the_scene_boundary_refuses_a_malformed_function_paint() {
        let mut builder = SceneBuilder::new();
        builder
            .fill(
                OutlineId(0),
                Affine::IDENTITY,
                FillRule::NonZero,
                function_paint(FunctionId(0)),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .expect("a well-formed function paint is an ordinary paint");

        let Paint::Function { program, range, .. } = function_paint(FunctionId(0)) else {
            unreachable!("the fixture is a function paint")
        };
        assert!(matches!(
            builder.fill(
                OutlineId(0),
                Affine::IDENTITY,
                FillRule::NonZero,
                Paint::Function {
                    program,
                    domain: unit_rect(),
                    matrix: Affine::scale(1.0, 0.0),
                    range,
                    background: None,
                },
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            ),
            Err(SceneError::SingularFunctionMatrix(_))
        ));
        assert_eq!(
            builder.finish().commands().len(),
            1,
            "the refused fill must not be appended"
        );
    }
}
