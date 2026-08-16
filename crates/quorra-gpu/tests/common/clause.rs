//! §11.4.6's line, written once: `P' = (1 − f) × P + S`.
//!
//! ISO 32000-2 §11.4.6 composites each element of a knockout group with the group's
//! *initial* backdrop and then takes a weighted average with the immediate backdrop,
//! weighted by the element's own source shape:
//!
//! > 𝛼gi = (1 − 𝑓si) × 𝛼gi−1 + 𝑓si × 𝛼t
//!
//! with the same weighting applied to colour. Where the group is isolated the initial
//! backdrop is transparent (§11.4.5) and §11.3.6's formula against `ab = 0` leaves
//! `co = as·Cs`, `ao = as` — the element's own premultiplied deposit. So the line every
//! caller of this module measures against is `P' = (1 − f) × P + S`, per channel and
//! premultiplied: `P` is what stood before the element, `f` is its **shape**
//! (§11.6.4.2: geometry) and `S` is its own premultiplied deposit.
//!
//! ADR 0025's staged pair is the same line asked for by name, which is why the four files
//! that measure it — `knockout_blend.rs`, `staged_compose.rs`, `function_knockout.rs` and
//! `function_staged.rs` — share this one arithmetic rather than four copies of it. It was
//! four copies until this module existed, three of them identical to the character;
//! `doc/HANDOVER.md` listed the merge and named its obstacle, which was that each copy
//! indexed its raster through its own file's `SIZE`.
//!
//! **The obstacle is removed rather than worked around**: the clause's line is stated over
//! every pixel of the rasters it is handed, and a raster's own length says how many that
//! is. No dimension is shared, so no file's probes are tied to another's.

/// One channel of a straight-alpha readback pixel, back in the premultiplied space
/// §11.3.6 and §11.4.6 are written in.
pub fn premul(raster: &[u8], at: usize, channel: usize) -> f32 {
    f32::from(raster[at + channel]) * f32::from(raster[at + 3]) / 255.0
}

/// Worst premultiplied deviation from §11.4.6's line over the whole raster, and how many
/// of its pixels are **partially covered**.
///
/// The second number is what says the fixture is worth measuring: a shape with no partially
/// covered pixel makes `f` either 0 or 1 everywhere, where a weighted average and a
/// composite agree for the wrong reason. Every caller asserts it, and each states its own
/// floor — the count is a property of that file's fixture, not of this arithmetic.
///
/// `before` is what stood before the element, `shape` carries `f` in its alpha, `deposit`
/// is the element drawn onto transparency, and `actual` is the frame under test. All four
/// are straight-alpha RGBA of the same dimensions; the comparison runs over as many pixels
/// as `actual` has.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "pixel indexing and the clause's own arithmetic, over rasters the caller just \
              drew"
)]
pub fn deviation_from_the_clause(
    before: &[u8],
    shape: &[u8],
    deposit: &[u8],
    actual: &[u8],
) -> (f32, u32) {
    let (mut worst, mut partial) = (0.0_f32, 0_u32);
    for at in (0..actual.len()).step_by(4) {
        let f = f32::from(shape[at + 3]) / 255.0;
        if f > 0.0 && f < 1.0 {
            partial += 1;
        }
        for channel in 0..3 {
            let expected =
                (1.0 - f).mul_add(premul(before, at, channel), premul(deposit, at, channel));
            worst = worst.max((premul(actual, at, channel) - expected).abs());
        }
    }
    (worst, partial)
}
