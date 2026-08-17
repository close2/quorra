//! Filling: flattened polylines become coverage bytes over a region of device pixels.
//!
//! One thing: the accumulation grid, and the two rules read off it. This is
//! [`raster`](super)'s definition of coverage executed — flattened edges deposit exact
//! signed trapezoid areas, a left-to-right prefix sum recovers the average winding per
//! pixel, and ISO 32000-2 §8.5.3.3's two rules turn a winding into a coverage byte.
//!
//! **The region is a window on one answer, not an answer of its own** (ADR 0049): an
//! edge that leaves the region is cut at the border and deposits its winding *there*,
//! which is what lets one rasterisation serve every tile cut out of it, and what makes
//! [`CoverageMask::crop`] a lookup rather than a second rasterisation. The three
//! deposit functions below are that one rule at three scales — a row slab, the borders
//! it crosses, and one single-cell trapezoid.

use super::flatten::Polyline;

/// A rasterised coverage tile: `width × height` bytes anchored at integer device
/// pixel `(left, top)`.
#[derive(Debug, Clone)]
pub(crate) struct CoverageMask {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    /// Row-major coverage bytes, `width × height`.
    pub coverage: Vec<u8>,
}

impl CoverageMask {
    /// A mask that admits nothing, over the given pixels.
    ///
    /// A legitimate mask rather than the absence of one: an empty clip region admits
    /// nothing *inside* it too, which is a different statement from having no clip, and
    /// both have tests (`doc/PLAN.md` §1.4).
    pub(crate) fn transparent(left: i32, top: i32, width: u32, height: u32) -> Self {
        Self {
            left,
            top,
            width,
            height,
            coverage: vec![0; (width as usize).saturating_mul(height as usize)],
        }
    }

    /// The window of this mask over another rectangle of device pixels, transparent
    /// wherever the two do not meet.
    ///
    /// **This is only a lookup because [`fill_mask`] cuts at its region's border** — the
    /// two share the pixel grid, and the same device pixel carries the same coverage
    /// whichever region computed it, to within the 1-of-255 rounding
    /// `a_tile_is_the_crop_of_the_region_that_contains_it` bounds. Outside this mask the
    /// answer is transparent by construction: a region is the intersection of its
    /// chain's bounds, and a closed path winds nothing beyond its own.
    // The corners are computed in `i64`, where an `i32` origin plus a `u32` extent cannot
    // wrap; every offset below is then a difference between two of those corners inside
    // the overlap, so it is non-negative and no larger than the smaller mask's extent.
    // Stated once here rather than at each of the six.
    #[allow(clippy::arithmetic_side_effects)]
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    pub(crate) fn crop(&self, left: i32, top: i32, width: u32, height: u32) -> Self {
        let mut cut = Self::transparent(left, top, width, height);
        let (ax0, ay0) = (i64::from(left), i64::from(top));
        let (ax1, ay1) = (ax0 + i64::from(width), ay0 + i64::from(height));
        let (bx0, by0) = (i64::from(self.left), i64::from(self.top));
        let (bx1, by1) = (bx0 + i64::from(self.width), by0 + i64::from(self.height));
        let (x0, y0) = (ax0.max(bx0), ay0.max(by0));
        let (x1, y1) = (ax1.min(bx1), ay1.min(by1));
        if x0 >= x1 || y0 >= y1 {
            return cut;
        }
        let (span, rows) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let (from_x, from_y) = ((x0 - bx0) as usize, (y0 - by0) as usize);
        let (into_x, into_y) = ((x0 - ax0) as usize, (y0 - ay0) as usize);
        for row in 0..rows {
            let from = (from_y + row) * self.width as usize + from_x;
            let into = (into_y + row) * width as usize + into_x;
            cut.coverage[into..into + span].copy_from_slice(&self.coverage[from..from + span]);
        }
        cut
    }
}

/// Which of ISO 32000-2 §8.5.3.3's two rules decides insideness.
///
/// `Hash` because it is part of the glyph cache's key: the same outline under the two
/// rules is two different pictures wherever a subpath nests (ADR 0024).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Rule {
    NonZero,
    EvenOdd,
}

/// Rasterise closed polylines into a coverage mask over the given integer pixel
/// region (`left..left+width`, `top..top+height`), by the module's stated definition.
///
/// Every subpath is treated as closed (fill semantics, ISO 32000-2 §8.5.3.1: filling
/// implicitly closes open subpaths).
///
/// **The region is a window on one answer, not an answer of its own.** Geometry outside
/// it is cut at the border and deposits its winding there ([`deposit_slab`]), so asking
/// for a tighter region returns what a wider one holds over the same pixels — to within
/// the accumulator's own rounding, which ADR 0049 measures at **1 of 255 on 2 pixels in
/// 2.9 million**. That is what lets one rasterisation of a clip's region serve every
/// tile cut out of it (`encode::residue`).
// The accumulation arithmetic below is bounded by construction: coordinates are
// clamped into the region, whose dimensions were checked against the frame budget
// before allocation. Stated once here rather than per line of a hot loop.
#[allow(clippy::arithmetic_side_effects)]
pub(crate) fn fill_mask(
    polylines: &[Polyline],
    rule: Rule,
    left: i32,
    top: i32,
    width: u32,
    height: u32,
) -> CoverageMask {
    let w = width as usize;
    let h = height as usize;
    // One spill column: a deposit at the right edge lands in it rather than wrapping.
    let mut acc = vec![0.0_f32; (w + 1) * h];

    #[allow(clippy::cast_precision_loss)] // region dims are bounded by target limits
    let (fw, fh) = (w as f32, h as f32);
    for polyline in polylines {
        let n = polyline.points.len();
        for i in 0..n {
            let p0 = polyline.points[i];
            // Filling closes every subpath: the last edge returns to the start.
            let p1 = polyline.points[(i + 1) % n];
            #[allow(clippy::cast_precision_loss)]
            let (x0, y0) = (p0.x - left as f32, p0.y - top as f32);
            #[allow(clippy::cast_precision_loss)]
            let (x1, y1) = (p1.x - left as f32, p1.y - top as f32);
            accumulate_edge(&mut acc, w, fw, fh, x0, y0, x1, y1);
        }
    }

    // Prefix-sum each row: the running total is the average winding per pixel; the
    // rule maps winding to coverage; `round` quantises (our stated rule, ADR 0005).
    let mut coverage = vec![0_u8; w * h];
    for y in 0..h {
        let mut running = 0.0_f32;
        for x in 0..w {
            running += acc[y * (w + 1) + x];
            let cov = match rule {
                Rule::NonZero => running.abs().min(1.0),
                Rule::EvenOdd => {
                    let m = running.abs().rem_euclid(2.0);
                    1.0 - (m - 1.0).abs()
                }
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                coverage[y * w + x] = (cov * 255.0).round() as u8;
            }
        }
    }
    CoverageMask {
        left,
        top,
        width,
        height,
        coverage,
    }
}

/// Deposit one edge's signed trapezoid areas into the accumulation grid.
///
/// The edge is split at every horizontal pixel row and every vertical pixel column
/// it crosses, so each piece lies within one cell; a piece from `(xs, ys)` to
/// `(xe, ye)` inside cell `k` deposits `d·(1 − xm)` into `k` and `d·xm` into `k+1`,
/// where `d` is the signed slab height and `xm` the piece's mean x within the cell —
/// the exact trapezoid area to the right of the edge, plus the spill that keeps the
/// running sum equal to the full winding beyond the crossing.
#[allow(clippy::arithmetic_side_effects, clippy::cast_possible_truncation)]
#[allow(clippy::cast_sign_loss, clippy::cast_precision_loss)]
// Two endpoints plus the grid: the coordinate bundle is the function's whole input,
// and a struct would only rename the eight numbers.
#[allow(clippy::too_many_arguments)]
fn accumulate_edge(
    acc: &mut [f32],
    w: usize,
    fw: f32,
    fh: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
) {
    // Exact comparison: a horizontal edge deposits nothing by definition, and a
    // nearly-horizontal one deposits its nearly-zero area correctly.
    #[allow(clippy::float_cmp)]
    if y0 == y1 {
        return;
    }
    let (dir, top_x, top_y, bot_x, bot_y) = if y0 < y1 {
        (1.0_f32, x0, y0, x1, y1)
    } else {
        (-1.0, x1, y1, x0, y0)
    };
    // Clip vertically to the region; x interpolates along the clipped span.
    let (top_x, top_y) = if top_y < 0.0 {
        (
            top_x + (bot_x - top_x) * (0.0 - top_y) / (bot_y - top_y),
            0.0,
        )
    } else {
        (top_x, top_y)
    };
    let (bot_x, bot_y) = if bot_y > fh {
        (top_x + (bot_x - top_x) * (fh - top_y) / (bot_y - top_y), fh)
    } else {
        (bot_x, bot_y)
    };
    if bot_y <= top_y {
        return;
    }
    let dxdy = (bot_x - top_x) / (bot_y - top_y);
    // **A slope this edge cannot state is a slab this edge cannot fill.** The numerator
    // is bounded by twice the largest device coordinate the scene contract admits
    // (`MAX_COORDINATE` on a point and on a transform coefficient, so `4e27`), and the
    // denominator is positive by the test above — so a non-finite ratio means the slab
    // is under `2.4e-11` of a pixel tall, and the exact area such an edge deposits is
    // under `2.4e-11` where one coverage step is `1/255`. Depositing nothing is the
    // right answer to eleven decimal places, and it is the only answer that keeps a NaN
    // out of the accumulator: a NaN survives the prefix sum, and `abs().min(1.0)`
    // returns **1.0** for it, so one such edge paints the rest of its row solid. The
    // same test also catches a NaN arriving from a coordinate that is not finite, which
    // `Device::render` refuses at the viewport before it can reach here.
    if !dxdy.is_finite() {
        return;
    }

    let mut y = top_y.floor().max(0.0);
    while y < bot_y {
        let row = y as usize;
        if row >= acc.len() / (w + 1) {
            break;
        }
        let entry_y = top_y.max(y);
        let exit_y = bot_y.min(y + 1.0);
        let entry_x = top_x + (entry_y - top_y) * dxdy;
        let exit_x = top_x + (exit_y - top_y) * dxdy;
        deposit_slab(
            &mut acc[row * (w + 1)..(row + 1) * (w + 1)],
            fw,
            dir,
            entry_x,
            entry_y,
            exit_x,
            exit_y,
        );
        y += 1.0;
    }
}

/// Deposit one row slab's areas. `xs`/`xe` are x at the slab's top and bottom; the part
/// of the slab spent left or right of the region is **cut off at the border** and
/// deposited there, rather than compressed into the columns inside it.
///
/// # Why the cut, and what clamping the endpoints instead used to cost (ADR 0049)
///
/// A slab piece running from `x = −25` to `x = +2` covers the region's first column for
/// most of its height and only reaches `x = 2` at the very end. Clamping the two
/// endpoints to `[0, fw]` and interpolating between them — which is what this function
/// did until ADR 0049 — spreads that height evenly from column 0 to column 2 instead.
/// The row's *total* winding survives (every column past the crossing reads the same
/// value, which is why nothing downstream ever saw it), but the columns at the border
/// get somebody else's share: **up to 185 of 255 on a shallow edge**, measured by
/// `a_tile_whose_geometry_enters_from_outside_is_exact`.
///
/// Cutting at the border is the same statement §10.7.4 makes about a clipping region —
/// the pixels a fill would cover — applied to the region this mask is asked for: what
/// lies outside contributes its winding, at the border, for exactly the height it spends
/// there.
///
/// The cut runs only when an endpoint is outside; a piece wholly inside takes the same
/// arithmetic it always did, to the bit, which is what keeps every tile that is not cut
/// by a clip or by the page edge pixel-for-pixel where it was.
#[allow(clippy::arithmetic_side_effects, clippy::cast_possible_truncation)]
#[allow(clippy::cast_sign_loss, clippy::cast_precision_loss)]
fn deposit_slab(row: &mut [f32], fw: f32, dir: f32, xs: f32, ys: f32, xe: f32, ye: f32) {
    if xs >= 0.0 && xs <= fw && xe >= 0.0 && xe <= fw {
        deposit_inside(row, fw, dir, xs, ys, xe, ye);
        return;
    }
    let (dx, dy) = (xe - xs, ye - ys);
    // At most two borders can be crossed, and `dx == 0` crosses neither: a vertical
    // piece is on one side for its whole height.
    let mut cuts = [(0.0_f32, 0.0_f32); 2];
    let mut count = 0;
    if dx != 0.0 {
        for border in [0.0_f32, fw] {
            let t = (border - xs) / dx;
            if t > 0.0 && t < 1.0 {
                cuts[count] = (t, border);
                count += 1;
            }
        }
        if count == 2 && cuts[1].0 < cuts[0].0 {
            cuts.swap(0, 1);
        }
    }
    // Each part is interpolated from the piece's own ends, so a cut cannot move where
    // the piece starts or finishes: the border's own x is used at the seam, and the
    // outer ends stay the values the caller passed.
    let (mut px, mut py) = (xs, ys);
    for (t, border) in cuts.iter().take(count).copied() {
        let (nx, ny) = (border, ys + dy * t);
        deposit_inside(row, fw, dir, px, py, nx, ny);
        (px, py) = (nx, ny);
    }
    deposit_inside(row, fw, dir, px, py, xe, ye);
}

/// One slab piece that does not cross the region's borders: split at each vertical cell
/// boundary it does cross, and deposit the exact trapezoid areas.
///
/// A piece wholly outside arrives here with both ends on the same side; the clamp then
/// collapses it onto the border column, which is where its winding belongs.
#[allow(clippy::arithmetic_side_effects, clippy::cast_possible_truncation)]
#[allow(clippy::cast_sign_loss, clippy::cast_precision_loss)]
fn deposit_inside(row: &mut [f32], fw: f32, dir: f32, xs: f32, ys: f32, xe: f32, ye: f32) {
    let xs = xs.clamp(0.0, fw);
    let xe = xe.clamp(0.0, fw);
    let (mut px, mut py) = (xs, ys);
    loop {
        // The next vertical boundary in the direction of travel, or the slab's end.
        let boundary = if xe > px {
            let b = px.floor() + 1.0;
            if b < xe { Some(b) } else { None }
        } else if xe < px {
            let b = px.ceil() - 1.0;
            if b > xe { Some(b) } else { None }
        } else {
            None
        };
        let (nx, ny) = match boundary {
            Some(b) => {
                let t = (b - xs) / (xe - xs);
                (b, ys + (ye - ys) * t)
            }
            None => (xe, ye),
        };
        // One single-cell piece: exact trapezoid deposit.
        let d = dir * (ny - py);
        if d != 0.0 {
            let xm = 0.5 * (px + nx);
            let cell = (xm.floor().max(0.0) as usize).min(row.len().saturating_sub(2));
            let frac = xm - cell as f32;
            row[cell] += d * (1.0 - frac);
            row[cell + 1] += d * frac;
        }
        if boundary.is_none() {
            break;
        }
        (px, py) = (nx, ny);
    }
}
