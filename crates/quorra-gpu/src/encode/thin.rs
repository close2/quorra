//! How thin a mark is, and the width below which the device lane stops being able to
//! promise ISO 32000-2 §10.7.4.
//!
//! The clause, verbatim, with the sentence that carries the reason:
//!
//! > A shape shall be scan-converted by painting any pixel whose half-open square region
//! > intersects the shape, no matter how small the intersection is. This ensures that no
//! > shape ever disappears as a result of unfavourable placement relative to the device
//! > pixel grid, as might happen with other possible scan conversion rules.
//!
//! The device lane of ADR 0016 counts samples on an ordered grid, so it answers *is this
//! sample point inside the shape* and never *does this shape intersect this pixel*. A
//! mark narrower than the grid's own step can therefore fall entirely between two
//! columns of sample points and read zero — which is the disappearance the sentence above
//! forbids, reached by the route it names. The processor lane computes the exact area
//! (ADR 0005) and cannot lose such a mark.
//!
//! So this module is the two numbers ADR 0070's fifth lane condition compares:
//! [`sample_column_spacing`], derived from the grid rather than written down, and
//! [`ThinAxis`], which is the one definition of *how thin a mark is* in this tree.

/// The distance between neighbouring columns of the device lane's sample grid, in
/// device pixels — and so the narrowest mark that lane can promise to draw wherever it
/// lands.
///
/// **Derived from the grid, not chosen.** `crate::winding::sample_offsets` lays `n`
/// samples on a `√n × √n` ordered grid and puts the k-th column at `(k + ½)/√n` of a
/// pixel. Consecutive columns are therefore `1/√n` apart, and so is the wrap across a
/// pixel boundary: the last column of one pixel sits `1/(2√n)` from its right edge and
/// the first column of the next sits `1/(2√n)` past it, because the grid is symmetric
/// about the pixel centre. The columns are one lattice of period `1/√n` across the whole
/// device, so a half-open interval of that length contains a column **wherever it
/// lands**, and one shorter than it has placements that contain none.
///
/// `tests/thin_marks.rs` holds both halves of that on the device rather than on paper:
/// `at_the_sample_spacing_the_device_lane_holds_the_clause_at_every_position` sweeps ten
/// sub-pixel positions at exactly this width, and the neighbouring test sweeps the same
/// positions below it.
///
/// **What the extremes of [`Options::coverage_samples`] make of it.** That field is
/// public and settable; construction rounds it down to a square and clamps it to
/// `4..=64`, so the grid's side is 2 to 8 and this is **0.5 at four samples** down to
/// **0.125 at sixty-four**. The condition therefore narrows as a caller buys quality,
/// which is the direction it should move in: more samples is a grid that misses fewer
/// marks, and so fewer marks that need the other lane.
///
/// [`Options::coverage_samples`]: crate::startup::Options::coverage_samples
#[allow(clippy::cast_precision_loss)] // a sample count clamped to 4..=64 at construction
pub(super) fn sample_column_spacing(samples: u32) -> f32 {
    1.0 / (samples.max(1) as f32).sqrt()
}

/// How narrow a mark is across its thinnest known axis, in device pixels.
///
/// **The one definition of "thin" in this tree**, and a newtype rather than an `f32` so
/// that the lane condition cannot be handed a tile side or a stroke width by accident.
///
/// # What it is measured from, and what that misses
///
/// A mark reaches [`Encoder::take_gpu_lane`](super::Encoder::take_gpu_lane) as device
/// geometry plus, for a stroke, the width §8.4.3 already resolved into device pixels
/// (§4.5 of the brief settles that upstream). Two bounds on its thickness follow, and
/// this is the smaller of the ones that apply:
///
/// - **the narrower side of its device box.** The mark lies inside that box, so across
///   that axis it is nowhere wider. Exact for the axis-aligned rule a document draws
///   most of its thin marks with.
/// - **a stroke's own resolved device width.** A stroke is nowhere wider than that
///   across its path, at any angle, so this catches a turned rule that the box does not
///   — which is why a stroke is measured by both and kept at the smaller.
///
/// **The residual, stated rather than discovered later: a filled hairline at 45°.** A
/// thin parallelogram given as a *fill* has a device box far wider than the mark and no
/// stroke width to be read instead, so it reads thick, keeps the device lane, and is not
/// diverted by ADR 0070's condition. It does not vanish there — it crosses many pixels
/// and catches a sample column in some of them — it **dots**: its coverage is uneven
/// along its length where the processor lane's would be even. The corpus says the
/// residual is small (`doc/notes-thin-mark-options.md` §2.4: the corpus's largest
/// sub-quarter-pixel stroke population is 29 375 marks on one page and exactly one of
/// them takes the device lane at all).
///
/// A second, quieter case of the same shape: a *curved* mark's box here is its control
/// hull's, which is an over-estimate for a curve, so a thin curve can read thicker than
/// it is. Same direction, same consequence, and the corpus's answer is the same one.
///
/// # Which way the error may point
///
/// Over-stating a mark's thinness costs it the device lane, which is CPU rasterisation
/// for a mark that would have been drawn correctly anyway — a cost. Under-stating it
/// leaves a mark on a lane that can lose it — a §10.7.4 violation. So every bound above
/// is an **upper** bound on the mark's thickness, and where two apply the smaller is
/// taken.
#[derive(Debug, Clone, Copy)]
pub(super) struct ThinAxis(f32);

impl ThinAxis {
    /// The thin axis of a mark with these device bounds — `(x0, y0, x1, y1)` — and, where
    /// the mark is a stroke, the width §8.4.3 resolved for it.
    ///
    /// `None` is a fill, whose box is the only bound there is.
    #[allow(clippy::arithmetic_side_effects)] // two subtractions of device coordinates,
    // each already bounded by `MAX_COORDINATE`; a non-finite difference is handled below
    pub(super) fn of(bounds: (f32, f32, f32, f32), stroke_width: Option<f32>) -> Self {
        let (x0, y0, x1, y1) = bounds;
        let across = (x1 - x0).min(y1 - y0);
        Self(match stroke_width {
            Some(width) => across.min(width),
            None => across,
        })
    }

    /// Whether the device lane's sample grid could miss this mark entirely.
    ///
    /// Strictly below the spacing: at exactly one column spacing the grid catches the
    /// mark wherever it lands, which is the arithmetic
    /// [`sample_column_spacing`] states and `tests/thin_marks.rs` measures.
    ///
    /// **A non-finite thin axis answers `false`**, because `f32` comparison against a
    /// `NaN` is false — the mark keeps whichever lane the four cost conditions choose,
    /// which is what it did before this condition existed. That is the right default:
    /// this condition exists to *decline* a lane, and a mark whose geometry is already
    /// beyond the encoder's coordinate ceiling is refused elsewhere by name rather than
    /// being routed on a comparison that means nothing.
    pub(super) fn can_fall_between_sample_columns(self, spacing: f32) -> bool {
        self.0 < spacing
    }
}

#[cfg(test)]
mod tests {
    use super::{ThinAxis, sample_column_spacing};

    /// The spacing at each sample count `Options::coverage_samples` can become, including
    /// both ends of the clamp — the numbers the condition's reach is stated in.
    #[test]
    fn the_spacing_is_one_over_the_grids_side_at_every_admitted_sample_count() {
        for (samples, side) in [(4, 2.0_f32), (9, 3.0), (16, 4.0), (36, 6.0), (64, 8.0)] {
            let spacing = sample_column_spacing(samples);
            assert!(
                (spacing - 1.0 / side).abs() <= f32::EPSILON,
                "{samples} samples sit on a {side} × {side} grid, so its columns are \
                 1/{side} apart; got {spacing}"
            );
        }
        // The two extremes the option admits, written out because they are what the
        // condition's reach is quoted as.
        assert!((sample_column_spacing(4) - 0.5).abs() <= f32::EPSILON);
        assert!((sample_column_spacing(64) - 0.125).abs() <= f32::EPSILON);
    }

    /// A stroke is measured by its own resolved device width as well as by its box, so a
    /// turned hairline — whose box is wide — is still read as thin.
    #[test]
    fn a_strokes_width_is_read_where_it_is_thinner_than_the_box() {
        let spacing = sample_column_spacing(16);
        let turned = (0.0, 0.0, 100.0, 100.0);
        assert!(
            !ThinAxis::of(turned, None).can_fall_between_sample_columns(spacing),
            "a fill with a 100-pixel box is not thin by any bound this type has — which \
             is the residual the type's own documentation states"
        );
        assert!(
            ThinAxis::of(turned, Some(0.1)).can_fall_between_sample_columns(spacing),
            "a stroke a tenth of a pixel wide is thin however its box is turned"
        );
        // And the box still decides where it is the narrower of the two: a wide stroke
        // clipped to a sliver of a tile is as losable as a thin one.
        assert!(
            ThinAxis::of((0.0, 0.0, 0.1, 100.0), Some(9.0))
                .can_fall_between_sample_columns(spacing),
            "the smaller of the two bounds is the one that decides"
        );
    }

    /// The boundary is *below*, not *at*: a mark exactly one column spacing wide catches
    /// a column wherever it lands, so it keeps the lane the cost conditions choose.
    #[test]
    fn a_mark_at_exactly_the_spacing_is_not_declined() {
        let spacing = sample_column_spacing(16);
        assert!(
            !ThinAxis::of((0.0, 0.0, spacing, 10.0), None)
                .can_fall_between_sample_columns(spacing),
            "at exactly one column spacing the grid cannot miss the mark"
        );
        assert!(
            ThinAxis::of((0.0, 0.0, spacing * 0.99, 10.0), None)
                .can_fall_between_sample_columns(spacing),
            "just below it, it can"
        );
    }

    /// Principle 3's rule at this seam: a thin axis that is not a number must not become
    /// a lane choice made on a comparison that means nothing.
    #[test]
    fn a_non_finite_thin_axis_declines_nothing() {
        assert!(
            !ThinAxis::of((0.0, 0.0, f32::NAN, 10.0), None)
                .can_fall_between_sample_columns(0.25),
            "a NaN extent answers false, leaving the four cost conditions to choose"
        );
    }
}
