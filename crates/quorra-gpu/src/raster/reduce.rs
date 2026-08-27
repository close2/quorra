//! The caller's image-filtering decisions, resolved here per placement (ADR 0089).
//!
//! ISO 32000-2 §8.9.5.3's `/Interpolate` rule and the caller's documented
//! area-averaging departure from §10.7.4 used to be settled upstream and baked into
//! the scene — which was correct at exactly one viewport, and cost the caller's
//! page-space scenes their survival on any page with a picture on it (their ADR
//! 0702's ledger). Under [`ImageFilter::Auto`](quorra_scene::ImageFilter) the flag
//! crosses the boundary instead, and this module answers the two questions where the
//! placement is known — the same amendment pattern as the stroke width (ADR 0085) and
//! the collapsed fill (ADR 0086), with the same containment: **every function here
//! mirrors the caller's `pdf_render` statement for statement** (`smoothed`, `factor`,
//! `Reduction`, `Bands`, `average_block`, `round_div` — their `paint.rs`), their CPU
//! oracle keeps the originals, and the cross-backend gates compare the two
//! continuously. The reduced samples are byte-identical by construction: the
//! arithmetic is integer sums and divisions with no float in the data path.
//!
//! What is deliberately *not* mirrored is their rayon split: a reduction here runs
//! once per `(image, factors)` for the device's life ([`crate::device`]'s cache),
//! against once per placement change upstream, so the parallel crossover their
//! `PARALLEL_FLOOR` earns has no work to divide. The cost is stated rather than
//! hidden: a 2700×3450 photograph pays ~20 ms once, on the first frame that minifies
//! it past a new integer factor.

use quorra_scene::ImageSpec;

/// Whether to filter between the samples of a `width` × `height` grid drawn under
/// `placement` — the caller's `smoothed`, statement for statement.
///
/// `placement` maps the unit square onto the device (§8.9.5.1), so the length of its
/// two columns is how many device pixels the image covers. `/Interpolate` true always
/// filters; otherwise a *magnified* image — a sample covering more than one device
/// pixel, the case §8.9.5.3 is about — is drawn as flat rectangles, and a reduced one
/// keeps the filter on (their ADR 0025 carries the §10.7.4 argument).
pub(crate) fn smoothed(width: u32, height: u32, interpolate: bool, placement: &[f32; 6]) -> bool {
    if interpolate {
        return true;
    }
    let across = length(placement[0], placement[1]);
    let down = length(placement[2], placement[3]);
    #[allow(clippy::cast_precision_loss)] // dimensions are far below f32's exact range
    let magnified = across > width as f32 || down > height as f32;
    !magnified
}

/// The reduced grid a placement asks for, or `None` where no device pixel gathers two
/// samples — the caller's `Image::reduction`, statement for statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Reduction {
    /// Whole samples averaged per output cell, per axis.
    pub factors: (u32, u32),
    /// The reduced grid.
    pub width: u32,
    pub height: u32,
    /// The reduced grid's own answer to the filter question, by the same rule.
    pub smoothed: bool,
}

/// See [`Reduction`]. `None` for an inconsistent spec exactly as theirs refuses one,
/// and the consistency question is asked **before** the factors for their stated
/// reason: `factor` clamps into `1.0 ..= width`, which a zero width makes panic.
pub(crate) fn reduction(
    spec: &ImageSpec,
    interpolate: bool,
    placement: &[f32; 6],
) -> Option<Reduction> {
    if !spec.is_consistent() {
        return None;
    }
    let factors = (
        factor(spec.width, length(placement[0], placement[1])),
        factor(spec.height, length(placement[2], placement[3])),
    );
    if factors.0 <= 1 && factors.1 <= 1 {
        return None;
    }
    let width = spec.width.div_ceil(factors.0);
    let height = spec.height.div_ceil(factors.1);
    Some(Reduction {
        factors,
        width,
        height,
        smoothed: smoothed(width, height, interpolate, placement),
    })
}

/// Averages each block of samples that would share one device pixel — the caller's
/// `Image::area_averaged`, statement for statement (premultiplied sums, proportional
/// band boundaries, round-to-nearest), minus the rayon split the module comment
/// accounts for. The bytes are theirs to the last one.
pub(crate) fn area_averaged(spec: &ImageSpec, reduced: Reduction) -> ImageSpec {
    let Reduction { width, height, .. } = reduced;
    let rows = Bands::new(spec.height, height);
    let columns = Bands::new(spec.width, width);
    let spans: Vec<(u32, u32)> = (0..width).map(|out_x| columns.at(out_x)).collect();
    let row_bytes = (width as usize).saturating_mul(4);
    let mut data: Vec<u8> = vec![0; row_bytes.saturating_mul(height as usize)];
    for (out_y, row) in data.chunks_exact_mut(row_bytes).enumerate() {
        let (y0, y1) = rows.at(u32::try_from(out_y).unwrap_or(u32::MAX));
        for (cell, &(x0, x1)) in row.chunks_exact_mut(4).zip(&spans) {
            cell.copy_from_slice(&average_block(spec, x0, y0, x1, y1));
        }
    }
    ImageSpec {
        width,
        height,
        data: data.into(),
    }
}

/// The caller's `factor`: how many source samples share a device pixel along one axis,
/// floored, clamped into the image.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)] // their casts, mirrored: dimensions are far below f32's exact range and the clamp
// bounds the result
fn factor(samples: u32, device: f32) -> u32 {
    if !device.is_finite() || device <= 0.0 {
        return 1;
    }
    ((samples as f32) / device)
        .floor()
        .clamp(1.0, samples as f32) as u32
}

/// The caller's `geom::length`.
fn length(dx: f32, dy: f32) -> f32 {
    (dx * dx + dy * dy).sqrt()
}

/// Proportional band boundaries — the caller's `Bands`, with their reason: fixed
/// multiples of the factor leave a short block at the edge occupying a whole output
/// cell, which squeezes the image into less than the unit square.
#[derive(Clone, Copy)]
struct Bands {
    samples: u64,
    cells: u64,
}

impl Bands {
    fn new(samples: u32, cells: u32) -> Self {
        Self {
            samples: u64::from(samples),
            cells: u64::from(cells.max(1)),
        }
    }

    fn at(self, index: u32) -> (u32, u32) {
        let edge = |i: u64| {
            let scaled = i.saturating_mul(self.samples).checked_div(self.cells);
            u32::try_from(scaled.unwrap_or(0).min(self.samples)).unwrap_or(u32::MAX)
        };
        let start = edge(u64::from(index));
        (start, edge(u64::from(index).saturating_add(1)).max(start))
    }
}

/// The caller's `average_block`: the mean of one block as straight-alpha RGBA8,
/// averaged premultiplied and divided back out, with their overflow argument (a block
/// holds at most `u32::MAX` samples of at most `255 × 255` each — under 2⁴⁸ in a u64).
#[allow(clippy::arithmetic_side_effects)] // bounded as the line above states, and
// mirrored: a saturating version here would be a different arithmetic than the oracle's
fn average_block(spec: &ImageSpec, x0: u32, y0: u32, x1: u32, y1: u32) -> [u8; 4] {
    let mut colour = [0u64; 3];
    let mut alpha_sum = 0u64;
    let mut count = 0u64;
    for y in y0..y1 {
        let row = (y as usize) * (spec.width as usize);
        let from = (row + x0 as usize) * 4;
        let to = (row + x1 as usize) * 4;
        let Some(span) = spec.data.get(from..to) else {
            continue;
        };
        for sample in span.chunks_exact(4) {
            let alpha = u64::from(sample[3]);
            for (sum, component) in colour.iter_mut().zip(sample) {
                *sum += u64::from(*component) * alpha;
            }
            alpha_sum += alpha;
            count += 1;
        }
    }
    if count == 0 || alpha_sum == 0 {
        return [0, 0, 0, 0];
    }
    let mut out = [0u8; 4];
    for (channel, sum) in out.iter_mut().zip(colour) {
        *channel = round_div(sum, alpha_sum);
    }
    out[3] = round_div(alpha_sum, count);
    out
}

/// The caller's `round_div`: round to nearest, clamp into a byte — truncation would
/// darken every reduced image by up to one level per component.
fn round_div(numerator: u64, denominator: u64) -> u8 {
    if denominator == 0 {
        return 0;
    }
    let rounded = numerator
        .saturating_add(denominator / 2)
        .checked_div(denominator)
        .unwrap_or(0);
    u8::try_from(rounded.min(u64::from(u8::MAX))).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn spec(width: u32, height: u32, data: Vec<u8>) -> ImageSpec {
        ImageSpec {
            width,
            height,
            data: Arc::from(data),
        }
    }

    /// A 4×4 checkerboard reduced twofold: every block holds two black and two white
    /// opaque samples, and their premultiplied mean is exactly 128 (255·2/4 → 127.5,
    /// rounded up) — the arithmetic asserted at the byte, which is the claim "mirrored
    /// statement for statement" has to cash.
    #[test]
    fn a_checkerboard_reduces_to_its_mean() {
        let mut data = Vec::new();
        for y in 0..4u32 {
            for x in 0..4u32 {
                let on = (x + y) % 2 == 0;
                let v = if on { 255 } else { 0 };
                data.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let source = spec(4, 4, data);
        let placement = [2.0, 0.0, 0.0, 2.0, 0.0, 0.0];
        let reduced = reduction(&source, false, &placement).expect("factor 2 both ways");
        assert_eq!(reduced.factors, (2, 2));
        assert_eq!((reduced.width, reduced.height), (2, 2));
        let out = area_averaged(&source, reduced);
        for cell in out.data.chunks_exact(4) {
            assert_eq!(cell, [128, 128, 128, 255]);
        }
    }

    /// The filter rule at both ends: magnified without `/Interpolate` is flat
    /// rectangles; at or below the image's own size the filter stays on; the flag
    /// always filters.
    #[test]
    fn the_filter_follows_the_clause() {
        assert!(!smoothed(8, 8, false, &[16.0, 0.0, 0.0, 16.0, 0.0, 0.0]));
        assert!(smoothed(8, 8, false, &[8.0, 0.0, 0.0, 8.0, 0.0, 0.0]));
        assert!(smoothed(8, 8, true, &[16.0, 0.0, 0.0, 16.0, 0.0, 0.0]));
    }

    /// A transparent sample pulls nothing: the mean is premultiplied, so a block of
    /// one opaque red and one transparent green is red at half alpha, not brown.
    #[test]
    fn transparency_carries_no_colour() {
        let data = vec![255, 0, 0, 255, 0, 255, 0, 0];
        let source = spec(2, 1, data);
        let out = area_averaged(
            &source,
            Reduction {
                factors: (2, 1),
                width: 1,
                height: 1,
                smoothed: true,
            },
        );
        assert_eq!(&out.data[..], &[255, 0, 0, 128]);
    }
}
