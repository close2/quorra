//! A colour ramp, sampled: the sweep between two stops, evaluated once on the CPU.
//!
//! ISO 32000-2 §8.7.4.5's axial and radial shadings are a colour function over a
//! parameter, and this device draws them by reading a table rather than by evaluating
//! one per pixel. The table's arithmetic is deliberately ours rather than the driver's
//! (ADR 0011): the shader reads it with `textureLoad` at a rounded index, so every
//! adapter gets the same texel, where filtering between two texels would not promise
//! that.
//!
//! Nothing here touches the GPU, which is why it is not in the file that owns one. The
//! texture these bytes become is [`super::textures`]'s, and which ramps a frame needs
//! at all is [`super::resident`]'s.

use quorra_scene::{Color, Stop};

/// Texels per sampled ramp. 4096 rather than the 256 first chosen: a ramp with
/// *hard* stop boundaries (a banded shading) has its boundaries snapped to this
/// grid, and a page-spanning axis divided by 510 was a visible ~3.5 px band
/// displacement on a real page (the corpus's `issue10572.pdf`). Divided by 8190
/// it is under an eighth of a pixel on the same page, for 16 KiB per resident
/// ramp — priced against the resource budget like everything else.
///
/// The snap is one-sided and strictly under one step: a boundary lands on the first
/// grid position **at or after** its offset, never before it. That follows from
/// [`ramp_color_at`] giving a bound to the interval that starts there
/// (ISO 32000-2 §7.10.4), and it is what the bound above is a bound *on* — before
/// ADR 0055 a boundary that fell exactly on a grid position was displaced a whole
/// step past it, which no resolution would have fixed.
pub(super) const RAMP_RESOLUTION: u32 = 4096;

/// Sample a validated ramp to [`RAMP_RESOLUTION`] straight-RGBA8 texels, on the
/// CPU (ADR 0011).
///
/// The shader indexes the result with `textureLoad` at `round(t·(N−1))`, reading
/// N from the texture itself, so the sweep's colour arithmetic is this
/// function's — deterministic across adapters — rather than the driver's
/// texture filtering.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // round of 0..=255
#[allow(clippy::cast_precision_loss)] // i < RAMP_RESOLUTION, far below 2^24
pub(super) fn sample_ramp(stops: &[Stop]) -> Vec<u8> {
    let entries = RAMP_RESOLUTION as usize;
    let mut out = Vec::with_capacity(entries.saturating_mul(4));
    let last = (RAMP_RESOLUTION.saturating_sub(1)) as f32;
    for i in 0..RAMP_RESOLUTION {
        let color = ramp_color_at(stops, i as f32 / last);
        for component in [color.r, color.g, color.b, color.a] {
            // Components were validated into 0..=1 at upload.
            out.push((component * 255.0).round() as u8);
        }
    }
    out
}

/// The ramp's colour at `t`: constant before the first and after the last stop, and
/// linearly interpolated between neighbours.
///
/// A ramp is a shading's colour function already sampled onto stops — ISO 32000-2
/// §8.7.4.5.3 for the axial case and §8.7.4.5.4 for the radial both say of `Domain`
/// that "[t]he variable t becomes the input argument to the colour function(s)" — and
/// the caller places *two stops at one offset* wherever that function jumps, which is a
/// §7.10.4 stitching bound. So this is the reconstruction of a stitching function, and
/// which side of a bound owns the bound is §7.10.4's to answer rather than ours.
///
/// Three clauses decide the three cases. Outside the stops, §7.10.1's Table 38 entry
/// for `Domain`:
///
/// > Input values outside the declared domain shall be clipped to the nearest boundary
/// > value.
///
/// Between two stops, §7.10.3's type 2 with the exponent of 1 that a colour ramp is:
///
/// > Each input value x shall return n values, given by yj = C0j + x N × (C1j − C0j),
/// > for 0 ≤ j < n.
///
/// On a bound, §7.10.4 (subscripts are set below the line in the original and written
/// inline here):
///
/// > The Bounds array shall describe a series of k half-open intervals, closed on the
/// > left and open on the right with the following exceptions:
/// >
/// > - the last interval, shall always be closed on the right,
/// > - if Domain0 = Bounds0 then the first interval shall be closed on both the left
/// >   and right and the second (next) interval shall be open on the left.
///
/// *Closed on the left* is why the loop below compares with `<` rather than `<=`: a `t`
/// exactly on a stop's offset belongs to the interval that **starts** there, so where
/// two stops share an offset the later one's colour applies at exactly that point. The
/// two exceptions are the two ends, and they do not point the same way:
///
/// - at the ramp's **first** offset the first interval is degenerate and closed on both
///   sides, so a coincident pair there takes the **earlier** stop's colour — which is
///   what the `t <= first.offset` below is, and it is the clause and not a convenience;
/// - at the **last** offset the last interval is closed on the right and the one before
///   it is open there, so a coincident pair takes the **later** stop's colour — which is
///   what falling out of the loop gives.
///
/// That these degenerate ends are defined at all, rather than merely not forbidden, is
/// §7.10.4 saying it a second time about the encoding: "If the last bound, Boundsk-2,
/// is equal to Domain1, then x ′ shall be defined to be Encode2(k-1). For the degenerate
/// case, if the first bound, Bounds0, is equal to Domain0 then x′ shall be defined to be
/// Encode0." — `Encode0` being the *first* subfunction's pair and `Encode2(k-1)` the
/// last's. ADR 0055 has the reasoning and the corpus round behind the comparison.
fn ramp_color_at(stops: &[Stop], t: f32) -> Color {
    // Upload refused empty ramps; transparent black would still be an honest
    // answer for one, not an approximation of anything.
    let Some(first) = stops.first() else {
        return Color::new(0.0, 0.0, 0.0, 0.0);
    };
    // §7.10.1's clipping below the first offset, and §7.10.4's second exception on it.
    if t <= first.offset {
        return first.color;
    }
    let mut previous = *first;
    for stop in stops.iter().skip(1) {
        if t < stop.offset {
            // `previous.offset <= t < stop.offset` here: the loop returns at the first
            // stop *above* `t`, so `previous` is never above it either. The span is
            // therefore strictly positive without asking the upload's validation for it,
            // and `u` stays in `0..1` — which is the half-open interval, in arithmetic.
            let span = stop.offset - previous.offset;
            let u = (t - previous.offset) / span;
            let mix = |a: f32, b: f32| a + (b - a) * u;
            return Color::new(
                mix(previous.color.r, stop.color.r),
                mix(previous.color.g, stop.color.g),
                mix(previous.color.b, stop.color.b),
                mix(previous.color.a, stop.color.a),
            );
        }
        previous = *stop;
    }
    // Above the last offset, §7.10.1 clips; on it, §7.10.4's last interval is closed on
    // the right, so a coincident pair at the end takes this later colour.
    previous.color
}

/// The sweep's arithmetic against the clauses that define it.
///
/// Every expected number below is derived from ISO 32000-2 and the comment above it says
/// from which clause — never from what this function returns, which would make the test
/// a record of the implementation rather than a check on it.
#[cfg(test)]
mod tests {
    use quorra_scene::{Color, Stop};

    use super::{RAMP_RESOLUTION, ramp_color_at, sample_ramp};

    const RED: Color = Color {
        r: 1.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    const BLUE: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 1.0,
        a: 1.0,
    };

    fn stop(offset: f32, color: Color) -> Stop {
        Stop { offset, color }
    }

    /// The last index `sample_ramp` divides by, and the grid every claim below is on.
    #[allow(clippy::cast_precision_loss)] // 4095, exact in f32
    const LAST: f32 = (RAMP_RESOLUTION - 1) as f32;

    /// Two stops are §7.10.3's type 2 exponential with `N` of 1, whose value is
    /// `C0 + x^N × (C1 − C0)` — linear, so a quarter of the way along is a quarter of
    /// the way between the colours, exactly.
    #[test]
    fn two_stops_are_the_type_2_interpolation_between_them() {
        let stops = [stop(0.0, RED), stop(1.0, BLUE)];
        assert_eq!(ramp_color_at(&stops, 0.0), RED);
        assert_eq!(ramp_color_at(&stops, 1.0), BLUE);
        assert_eq!(
            ramp_color_at(&stops, 0.25),
            Color::new(0.75, 0.0, 0.25, 1.0)
        );
        assert_eq!(ramp_color_at(&stops, 0.5), Color::new(0.5, 0.0, 0.5, 1.0));
    }

    /// §7.10.1 clips an input outside a function's declared domain to the nearest
    /// boundary value, so a ramp whose stops span only the middle of `0..=1` holds its
    /// end colours over the rest rather than fading out of them.
    #[test]
    fn outside_the_stops_a_ramp_holds_its_end_colours() {
        let stops = [stop(0.25, RED), stop(0.75, BLUE)];
        assert_eq!(ramp_color_at(&stops, 0.0), RED);
        assert_eq!(ramp_color_at(&stops, 0.25), RED);
        assert_eq!(ramp_color_at(&stops, 0.75), BLUE);
        assert_eq!(ramp_color_at(&stops, 1.0), BLUE);
    }

    /// A coincident pair of offsets is §7.10.4's stitching boundary: two subfunctions
    /// meet at a bound and neither interpolates across it, so the two sides are the two
    /// colours and there is nothing between them.
    ///
    /// **The bound itself is the later subfunction's**, because §7.10.4's subdomains are
    /// "closed on the left and open on the right" away from the two ends — so `t == 0.5`
    /// is in `[0.5, 1.0]` and not in `[0.0, 0.5)`, and the colour there is `BLUE`.
    #[test]
    fn a_coincident_pair_is_a_step_and_not_a_ramp() {
        let stops = [
            stop(0.0, RED),
            stop(0.5, RED),
            stop(0.5, BLUE),
            stop(1.0, BLUE),
        ];
        // One grid step either side of the boundary: the two subfunctions' own values,
        // not a blend of them.
        assert_eq!(ramp_color_at(&stops, 0.5 - 1.0 / LAST), RED);
        assert_eq!(ramp_color_at(&stops, 0.5 + 1.0 / LAST), BLUE);
        // And the bound belongs to the interval that starts there.
        assert_eq!(ramp_color_at(&stops, 0.5), BLUE);
        // And each subfunction is constant over its own subdomain, which is what makes
        // the step a step: §7.10.3's `C0 == C1` is a type 2 that does not move.
        assert_eq!(ramp_color_at(&stops, 0.1), RED);
        assert_eq!(ramp_color_at(&stops, 0.9), BLUE);
    }

    /// §7.10.4's two exceptions to *closed on the left*, which point opposite ways and
    /// are the only two places a ramp's boundary rule reverses:
    ///
    /// > - the last interval, shall always be closed on the right,
    /// > - if Domain0 = Bounds0 then the first interval shall be closed on both the left
    /// >   and right and the second (next) interval shall be open on the left.
    ///
    /// So a pair coincident at the ramp's **first** offset gives that point to the
    /// earlier stop — the degenerate first interval `[0, 0]` — and a pair coincident at
    /// its **last** offset gives it to the later one, whose interval `[1, 1]` is closed
    /// on the right while the one below it is open there. Both are grid positions
    /// `sample_ramp` visits exactly, so both are texels of every such ramp.
    #[test]
    fn the_ramps_two_ends_take_a_coincident_pair_opposite_ways() {
        let at_the_start = [stop(0.0, RED), stop(0.0, BLUE), stop(1.0, BLUE)];
        assert_eq!(ramp_color_at(&at_the_start, 0.0), RED);
        let at_the_end = [stop(0.0, RED), stop(1.0, RED), stop(1.0, BLUE)];
        assert_eq!(ramp_color_at(&at_the_end, 1.0), BLUE);
        // Each is one texel of the table and not a region of it: the neighbour on the
        // inside is the other subfunction's colour in both cases.
        assert_eq!(ramp_color_at(&at_the_start, 1.0 / LAST), BLUE);
        assert_eq!(ramp_color_at(&at_the_end, 1.0 - 1.0 / LAST), RED);
    }

    /// The table is one texel per grid step, ending on the last stop, with each
    /// component the 8-bit level nearest the colour — the arithmetic ADR 0011 keeps on
    /// the CPU so that every adapter reads the same texel.
    #[test]
    fn the_table_is_one_texel_per_grid_step() {
        let stops = [stop(0.0, RED), stop(1.0, BLUE)];
        let bytes = sample_ramp(&stops);
        assert_eq!(bytes.len(), RAMP_RESOLUTION as usize * 4);
        assert_eq!(&bytes[0..4], &[255, 0, 0, 255]);
        assert_eq!(&bytes[bytes.len() - 4..], &[0, 0, 255, 255]);
        // Texel `i` is the ramp at `i / (N − 1)`: a quarter of the way along the grid is
        // §7.10.3's quarter, rounded to a byte — `round(0.75 × 255)` is 191.
        let quarter = (RAMP_RESOLUTION as usize - 1) / 4;
        assert_eq!(&bytes[quarter * 4..quarter * 4 + 4], &[191, 0, 64, 255]);
    }

    /// What [`RAMP_RESOLUTION`] buys, as a bound rather than as a size: a hard boundary
    /// is snapped to the grid, and the grid is fine enough that the snap moves it by
    /// less than one step of the shading's parameter.
    ///
    /// The constant's own comment is the claim being checked — 256 texels displaced a
    /// band by ~3.5 px on a page-spanning axis, and this resolution keeps it under an
    /// eighth of a pixel. A resolution lowered without that being noticed fails here.
    ///
    /// The bound is one-sided because §7.10.4's interval is closed on the left: the
    /// boundary lands on the first grid position **at or after** its offset, and
    /// strictly under one step past it.
    #[test]
    fn a_hard_boundary_lands_within_one_grid_step_of_its_offset() {
        // Deliberately not a multiple of the grid step, so the boundary falls between
        // two texels and the snapping is what is being measured.
        let boundary = 0.313_79_f32;
        let stops = [
            stop(0.0, RED),
            stop(boundary, RED),
            stop(boundary, BLUE),
            stop(1.0, BLUE),
        ];
        let bytes = sample_ramp(&stops);
        let first_blue = bytes
            .chunks_exact(4)
            .position(|texel| texel == [0, 0, 255, 255])
            .expect("the ramp's second half is blue");
        #[allow(clippy::cast_precision_loss)] // an index below 4096
        let placed = first_blue as f32 / LAST;
        assert!(
            placed >= boundary && placed - boundary < 1.0 / LAST,
            "the boundary at {boundary} was placed at {placed}, not on the first grid \
             position at or after it"
        );
    }

    /// The texel a boundary *on* the grid falls in, which is where §7.10.4 and this
    /// table can disagree at all.
    ///
    /// Every other offset puts the bound strictly between two grid positions, so no
    /// sample asks the question; an offset of exactly `2048/4095` is asked by texel 2048
    /// and by nothing else. Closed on the left makes that texel the later subfunction's,
    /// and it is the witness ADR 0055 was written from — it read red before.
    #[test]
    fn a_boundary_on_the_grid_gives_its_texel_to_the_later_subfunction() {
        let boundary = 2048.0 / LAST;
        let stops = [
            stop(0.0, RED),
            stop(boundary, RED),
            stop(boundary, BLUE),
            stop(1.0, BLUE),
        ];
        let bytes = sample_ramp(&stops);
        assert_eq!(&bytes[2047 * 4..2047 * 4 + 4], &[255, 0, 0, 255]);
        assert_eq!(&bytes[2048 * 4..2048 * 4 + 4], &[0, 0, 255, 255]);
    }

    /// An empty ramp is refused at upload (`ResourceProblem::RampEmpty`), and this
    /// function still answers rather than dividing by a span it does not have:
    /// transparent black over the whole table, which is a colour and not a NaN.
    #[test]
    fn an_empty_ramp_samples_to_transparency() {
        let bytes = sample_ramp(&[]);
        assert_eq!(bytes.len(), RAMP_RESOLUTION as usize * 4);
        assert!(bytes.iter().all(|byte| *byte == 0));
    }
}
