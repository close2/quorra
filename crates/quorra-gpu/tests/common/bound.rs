//! **ADR 0006's cross-implementation bound, read at one pixel** — and the worst pixel of
//! a raster that exceeds it.
//!
//! One responsibility, and it is arithmetic: given what a CPU reference stored to a pixel
//! and what alpha the two sides came to there, how far apart may two conforming
//! rasterisers be at that pixel? Nothing here draws, nothing here asserts, and nothing
//! here knows a fixture.
//!
//! # Why this is shared where a tolerance is not
//!
//! [`super`]'s rule is *"a measurement is shared; a claim about a fixture is not"*, and
//! this module is the first thing in the suite that is squarely on the shared side of it.
//! A `const UNORM_TOLERANCE: i32 = 2` is a claim about a fixture — about its worst pixel,
//! at one scale, with one command list — and two files that state one cannot share it
//! without one of them asserting something it did not measure. [`bound_at`] is not that:
//! both of its inputs are properties of the pixel it is called for, so it is the same
//! function for every fixture, every scale and every command list, and a second copy of
//! it could only ever be a chance to have two.
//!
//! ADR 0072 built this arithmetic inside `m1.rs` and recorded, in the doc comment of
//! `bound_at` itself, that `m3.rs` *"states its own bound and should keep doing so"*. That
//! sentence was true about the constant `m3.rs` then held and is not true about this
//! function; ADR 0077 is where it is overturned, and the short form is that the seam runs
//! between a claim and an arithmetic rather than between two files.
//!
//! # The bound
//!
//! ADR 0006 states ±1 unorm step per blend stage in premultiplied space. A stage is one
//! float→unorm8 conversion, which happens once per command that covers the pixel — so the
//! number of *stores* is the multiplier. The device hands back straight alpha (§3), and
//! the conversion `straight = premultiplied · 255 / α` amplifies each of those steps by
//! `255/α` on the three colour channels. Alpha itself is stored straight and is never
//! amplified.
//!
//! ```text
//! bound(colour channel) = ceil(stores × 255 / α)
//! bound(alpha  channel) = stores
//! ```

/// What a CPU reference rasterised: its straight-alpha bytes, and **how many times each
/// pixel was stored**.
///
/// The second field is not decoration. It is the multiplier of the bound at that pixel
/// (see this module's header) and it is a fact only the rasteriser knows — no amount of
/// reading the two rasters recovers it. Counting it is what lets [`bound_at`] be derived
/// rather than typed, at any scale and for any scene a reference can draw.
pub struct Reference {
    /// Straight-alpha RGBA, as [`quorra_gpu::Raster`] hands back.
    pub pixels: Vec<u8>,
    /// Commands that stored to each pixel, in the same order as `pixels`' four-byte groups.
    ///
    /// Signed because every use of it is one side of a difference of bytes, and a count
    /// that has to be cast at each use is a cast that can be wrong at one of them.
    pub stores: Vec<i32>,
}

/// The bound at one pixel: `ceil(stores × 255 / alpha)`, and `0` where nothing stored.
///
/// Both inputs are properties of the pixel rather than of the fixture, which is what makes
/// this derivable instead of typed: `stores` is what the reference counted (see
/// [`Reference`]), and `alpha` is what the two sides agree the pixel's coverage came to.
///
/// **A pixel nothing stored to must agree exactly.** A device that inks where the
/// reference does not is not a rounding difference — it is a mark drawn outside the region
/// that admitted it — and §3's "transparent is `[0, 0, 0, 0]`" is what makes that
/// checkable. This is the clause `m3.rs` leans on hardest: a clip that leaks admits ink at
/// pixels whose store count is zero, and a fixture-wide tolerance is exactly the slack
/// that hides it.
pub fn bound_at(alpha: u8, stores: i32) -> i32 {
    if alpha == 0 {
        return 0;
    }
    let alpha = i32::from(alpha);
    // Rounded up: the bound is what the conversion can produce, and a fractional step is
    // a whole one once it lands in a byte.
    (stores * 255 + alpha - 1) / alpha
}

/// The worst pixel at which `actual` differs from `reference` by more than [`bound_at`]
/// allows, described in the terms an assertion needs — or `None` if none does.
///
/// "Worst" is by how far the difference **exceeds its own bound**, not by the raw
/// difference: a 3-step difference at α = 24 is inside the amplification and a 3-step one
/// at α = 255 is not, and the pixel worth naming in the panic is the second.
pub fn disagreement(actual: &[u8], reference: &Reference, width: u32) -> Option<String> {
    assert_eq!(
        actual.len(),
        reference.pixels.len(),
        "the device and the reference rasterised different numbers of pixels"
    );
    let mut worst: Option<(usize, usize, i32, i32)> = None;
    for (index, stores) in reference.stores.iter().enumerate() {
        let (got, want) = (
            &actual[index * 4..index * 4 + 4],
            &reference.pixels[index * 4..index * 4 + 4],
        );
        // The smaller of the two alphas is the one that amplifies more, so it is the one
        // the bound is read at: taking the reference's alone would let a device that
        // rounded α down claim the slack of an α it did not produce.
        let colour = bound_at(got[3].min(want[3]), *stores);
        for channel in 0..4 {
            // The alpha channel is stored straight and so is never amplified: it carries
            // the per-store bound itself.
            let bound = if channel == 3 { *stores } else { colour };
            let excess = (i32::from(got[channel]) - i32::from(want[channel])).abs() - bound;
            if excess > 0 && worst.is_none_or(|(_, _, previous, _)| excess > previous) {
                worst = Some((index, channel, excess, bound));
            }
        }
    }
    let (index, channel, excess, bound) = worst?;
    let (x, y) = (index as u32 % width, index as u32 / width);
    Some(format!(
        "at ({x}, {y}) channel {channel}: got {:?}, expected {:?} — {} unorm steps past a \
         bound of {bound} ({} stores at α {})",
        &actual[index * 4..index * 4 + 4],
        &reference.pixels[index * 4..index * 4 + 4],
        excess + bound,
        reference.stores[index],
        actual[index * 4 + 3].min(reference.pixels[index * 4 + 3]),
    ))
}
