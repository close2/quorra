//! What is read back off a rendered rule, and the two numbers §31's tables are stated in.
//!
//! Everything here is arithmetic on a raster this crate produced; nothing here renders.
//! The exact values it is compared against — `expected_centroid` is the band's own
//! midpoint, `expected_ink` its area — are arithmetic too and not another rasterisation,
//! which is what makes this a check against ADR 0005's area rule rather than a comparison
//! of two implementations.

use quorra_gpu::{Device, Target, Viewport};
use quorra_scene::{Affine, Scene};

use super::fixture::{ACROSS, ALONG, Case, WINDOW, rule, target};

/// One lane's answer for one placement.
pub(crate) struct Reading {
    /// The centroid of the rule's coverage along the axis it was swept in, in device
    /// pixels.
    pub(crate) centroid: f64,
    /// The rule's total coverage, in units of a fully covered pixel — its *ink*.
    pub(crate) ink: f64,
    /// Which of the encoder's lanes made the coverage, so that a number is never
    /// attributed to a lane the mark did not take.
    ///
    /// **`path` does not distinguish the two rasterisers, and reading it as though it did
    /// cost a round.** `LaneCounts::path` counts the processor's tiles and the winding
    /// lane's together — deliberately, because "they produce the same tile on the same
    /// sheet and differ only in who drew it" — so this field says `path` for both. The
    /// revision of this instrument before 2026-08-23 concluded from that agreement that a
    /// stroked hairline "takes the path lane under both settings", when its sampled column
    /// was the winding lane and snapping to the ¼ grid; the lane *name* agreed while the
    /// rasterisers did not (ADR 0076).
    ///
    /// So which of the two answered is read off the **pixels** here, never off this field:
    /// `main::assert_grid` requires the ink or the placement to be out by a quarter of the
    /// grid's own pitch, which the processor lane never is.
    pub(crate) lane: &'static str,
    /// The coverage of each lane of the raster across the swept axis, in units of a fully
    /// covered pixel — what the two numbers above are computed from, kept so that a
    /// caller can print the row split their §31.2 tables are written as.
    pub(crate) profile: Vec<f64>,
}

/// One position of one lane: build the rule, render it, read it back.
pub(crate) fn read(device: &mut Device, centre: f32, case: Case) -> Reading {
    let scene = rule(device, centre, case);
    read_scene(device, &scene, case, Window::PastTheDecoy)
}

/// Which lanes of the profile are read.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Window {
    /// Discard everything below [`WINDOW`], where a decoy placement sits.
    PastTheDecoy,
    /// Read the whole target — the graph paper's rules are spread across all of it.
    Whole,
}

/// Renders `scene` and reads its coverage profile back.
pub(crate) fn read_scene(
    device: &mut Device,
    scene: &Scene,
    case: Case,
    window: Window,
) -> Reading {
    let (width, height) = target(case.horizontal);
    let frame = device
        .render(
            scene,
            &Viewport::full(width, height, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("a hairline is within every budget");
    let lanes = frame.counters().lanes;
    let lane = match (lanes.rectangle, lanes.glyph, lanes.path) {
        (0, 0, _) => "path",
        (0, _, 0) => "glyph",
        (_, 0, 0) => "rect",
        _ => "mixed",
    };
    let pixels = frame
        .into_raster()
        .expect("a Readback frame carries a raster")
        .into_pixels();
    let profile = profile(&pixels, case.horizontal, window);
    let (centroid, ink) = centroid_and_ink(&profile);
    Reading {
        centroid,
        ink,
        lane,
        profile,
    }
}

/// The coverage of each row (or column) of the raster, in units of a fully covered pixel,
/// summed across the other axis.
#[allow(clippy::cast_precision_loss)] // a target dimension, far below f32's exact range
fn profile(pixels: &[u8], horizontal: bool, window: Window) -> Vec<f64> {
    let (width, height) = target(horizontal);
    let mut lanes = vec![0.0_f64; ACROSS as usize];
    for y in 0..height {
        for x in 0..width {
            let alpha = pixels[((y * width + x) * 4 + 3) as usize];
            let index = if horizontal { y } else { x } as usize;
            lanes[index] += f64::from(alpha) / 255.0;
        }
    }
    // Per pixel of the rule's length, so the numbers are coverages and not areas.
    for lane in &mut lanes {
        *lane /= f64::from(ALONG);
    }
    // **The window, and why it is in the profile rather than in the sum.** A decoy
    // placement (see `fixture::rule`) is a second rule this measurement must not see, and
    // cutting it out of the *sum* cuts the measured rule too when the two are near a
    // boundary — which is what an earlier version of this instrument did, reporting a
    // half-pixel "error" that was its own window truncating the mark. The decoy sits below
    // `WINDOW` and the measured rule well above it, so discarding whole lanes is exact.
    if window == Window::PastTheDecoy {
        for lane in lanes.iter_mut().take(WINDOW as usize) {
            *lane = 0.0;
        }
    }
    lanes
}

/// The centroid of a coverage profile, and its total — the two numbers the caller's §31
/// tables are stated in.
#[allow(clippy::cast_precision_loss)] // a lane index below 65 536
pub(crate) fn centroid_and_ink(profile: &[f64]) -> (f64, f64) {
    let ink: f64 = profile.iter().sum();
    if ink == 0.0 {
        return (f64::NAN, 0.0);
    }
    let weighted: f64 = profile
        .iter()
        .enumerate()
        // The pixel's own centre: a pixel wholly covered contributes its middle, which is
        // what makes a band from 16.0 to 17.0 read as 16.5 rather than as 16.
        .map(|(index, coverage)| (index as f64 + 0.5) * coverage)
        .sum();
    (weighted / ink, ink)
}

/// The centroid and ink of one rule of a profile holding several, taken over the lanes
/// within `reach` of where the geometry puts it.
///
/// A window again rather than a peak search, and whole lanes again: the graph paper's
/// pitch is sixteen pixels and a rule is two, so no rule can reach another's half and the
/// window truncates nothing.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn one_rule(profile: &[f64], centre: f64, reach: f64) -> (f64, f64) {
    let low = (centre - reach).max(0.0) as usize;
    let high = ((centre + reach) as usize).min(profile.len());
    let mut window = vec![0.0; profile.len()];
    window[low..high].copy_from_slice(&profile[low..high]);
    centroid_and_ink(&window)
}

/// How many lanes the exact profile inks that the sampled profile leaves at **zero**.
///
/// This is ISO 32000-2 §10.7.4's first sentence, counted rather than argued about:
///
/// > A shape shall be scan-converted by painting any pixel whose half-open square region
/// > intersects the shape, no matter how small the intersection is.
///
/// A lane the processor lane inked is a lane the shape reaches — that lane's exact area
/// inside the shape is not zero — so a zero there is a pixel the shape intersects and the
/// device did not paint. Answering with a count and not a bound is deliberate: the clause
/// admits no bound, and one such pixel is the whole of the finding.
///
/// **A lower bound, and deliberately the conservative one.** The reference is itself stored
/// at eight bits, so a lane whose exact area is under half a level reads zero there too and
/// is not counted, even though the shape reaches it. Erring this way is the right direction
/// for a number that says a clause is not met.
pub(crate) fn unpainted_lanes(exact: &[f64], sampled: &[f64]) -> usize {
    // `<= 0.0` rather than `== 0.0`: coverage is a sum of non-negative alphas, so the two
    // say the same thing about this data, and only one of them is a float equality.
    exact
        .iter()
        .zip(sampled)
        .filter(|(reference, drawn)| **reference > 0.0 && **drawn <= 0.0)
        .count()
}

/// The first two lanes of a profile that carry ink, as `(index, first, second)` — the
/// shape the caller's §31.2 row tables are written in ("row 141 / row 142 / total").
pub(crate) fn row_split(profile: &[f64]) -> (usize, f64, f64) {
    let first = profile.iter().position(|lane| *lane > 0.0).unwrap_or(0);
    let second = profile.get(first + 1).copied().unwrap_or(0.0);
    (first, profile.get(first).copied().unwrap_or(0.0), second)
}
