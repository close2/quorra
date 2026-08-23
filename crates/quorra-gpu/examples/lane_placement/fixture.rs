//! The pictures this instrument draws, the devices it draws them on, and the one
//! arithmetic that decides whether a picture reaches the lane it was built for.
//!
//! Every constant here is either a number the caller's `QUORRA_FEEDBACK.md` §31 states
//! or a **lane condition wearing a canvas's clothes** — see [`ALONG`], which is the
//! second kind and is why the previous revision of this instrument could not reach the
//! sampled lane at all.

use quorra_gpu::{Coverage, Device, Options};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, LineCap, LineJoin, Paint, Point, Scene,
    SceneBuilder, Segment, Stroke,
};

/// The target's extent **along** the rule, in device pixels.
///
/// **This is a lane condition, not a canvas size, and it is the correction this round
/// was opened for.** `Encoder::triangles_under_coverage` — ADR 0026's byte comparison,
/// which ADR 0075 split out of `take_gpu_lane` and did not otherwise change — is
/// `area >= triangle_bytes`: a mark takes the device lane only when its **own device
/// box**, `tile_side` over the unclipped bounds and not the visible tile, holds more
/// texels than its triangles hold bytes. A stroke's expansion is four points, so four triangles
/// (`append_polyline_triangles` fans one per edge), so
/// `4 × 3 × WindingVertex::STRIDE` = **384 bytes**; a fill of the six-point band below is
/// six, so **576**. A rule one device pixel thick has a box `length × 2`, so a mark
/// shorter than **192** device pixels can never take the sampled lane, and one shorter
/// than **288** can never take it as a fill — whatever `Coverage::Gpu` says.
///
/// The rule here runs past both edges of the target, so its length is `2 × (ALONG + 4)`
/// and its box is `4 × (ALONG + 4)` texels. **512 clears both floors with margin. The
/// previous revision's 128 clears exactly one of them**, at `4 × 132` = 528 texels: the
/// fill is declined at 528 against 576 — which is the "six-triangle band of 528 texels"
/// `doc/notes-glyph-phase-carry.md` §3 records — and **the stroke is not declined at
/// all**, 528 against 384. That round therefore had a hairline on the sampled lane and
/// read it as the processor's, because `LaneCounts::path` is the name of both
/// rasterisers. Measured at `ALONG` = 128 by this instrument before it was set back to
/// 512: the stroke row's sampled column snaps to the ¼ grid at ±0.1071 while its
/// processor column is exact to 0.0019.
pub(crate) const ALONG: u32 = 512;

/// The target's extent **across** the rule, in device pixels — the axis the sweep moves
/// in and the profile is read along.
///
/// Small on purpose and unchanged from the square this instrument used to draw on: it
/// carries [`BASE`], [`WINDOW`] and [`DECOY_GAP`], and every pixel of it costs the
/// software adapter time that [`ALONG`] already spends.
pub(crate) const ACROSS: u32 = 128;

/// An atlas eight of a hairline's tiles cannot fit, in bytes — see [`device`].
pub(crate) const TINY_ATLAS: u64 = 1024;

/// The rule's device width where the question is *placement*: the caller's population is
/// "axis-aligned rules about one device pixel wide" (§31.2).
///
/// **A multiple of every sample pitch this instrument uses**, which is exactly why it
/// cannot be the width the *ink* question is asked at — see [`WITNESS_WIDTH`].
pub(crate) const HAIRLINE: f32 = 1.0;

/// The rule's device width where the question is *ink*: the total coverage the caller's
/// `issue16500.pdf` witness states, 0.439 + 0.439 (§31.2).
///
/// **Not a multiple of any sample pitch**, and that is the whole point. The device
/// lane's samples lie on a lattice of period `1/√n` down the pixel, so a band whose
/// height is a multiple of that period contains the same number of lattice points
/// wherever it lands and loses no ink at any position — a sweep at [`HAIRLINE`] measures
/// the fixed points of the quantiser and reports zero, which is the trap
/// `doc/notes-glyph-phase-carry.md` §2 names in the other axis.
pub(crate) const WITNESS_WIDTH: f32 = 0.878;

/// The caller's own witness CTM: `bug1743245.pdf`'s graph paper is drawn under a uniform
/// scale of this, one `q … cm … S … Q` per rule (their §31.2).
pub(crate) const CTM: f32 = 0.317_180_62;

/// Where the rule sits, before the sub-pixel offset is added: far enough from the edges
/// that neither cap nor target boundary is in the measurement.
pub(crate) const BASE: f32 = 80.0;

/// Lanes below this are not read: it is where a decoy placement is put and the measured
/// rule is not (see `measure::profile`).
pub(crate) const WINDOW: u32 = 60;

/// How far a decoy placement sits from the measured rule, in device pixels — clear of
/// [`WINDOW`] on the other side, so neither mark can reach the other's half.
const DECOY_GAP: f32 = 40.0;

/// The target this case is drawn into, as `(width, height)` — long along the rule and
/// short across it, which is [`ALONG`]'s condition met in the cheapest pixels.
pub(crate) const fn target(horizontal: bool) -> (u32, u32) {
    if horizontal {
        (ALONG, ACROSS)
    } else {
        (ACROSS, ALONG)
    }
}

/// A device on the named coverage setting, with the named sample count, and optionally
/// with an atlas too small to hold a rule's tile.
///
/// **What reaches the sampled lane, and the claim this comment used to make.** Until
/// 2026-08-23 this comment said the small atlas "is what reaches the sampled lane at
/// all", because the lane chooser declines the device lane for anything `worth_caching`
/// (`Encoder::gpu_lane_admissible` since ADR 0075's split; `take_gpu_lane` before it).
/// The caller corrected that in their §37.4 and the source agrees: **it is true of a
/// solid fill and false of a stroke.** `Encoder::push_coverage_styled` passes
/// `CacheProspect::TooLarge` at the call site — "the atlas caches outlines by key, not
/// polylines" — so `worth_caching()` is `false` by construction for every stroke and
/// declines nothing. `Encoder::fill_solid` is where the atlas is asked for real, and
/// there the sentence holds.
///
/// What actually kept this instrument's hairline off the sampled grid was the **triangle
/// floor**, `area >= triangle_bytes`, which is [`ALONG`]'s subject and a different
/// condition with a different fix.
///
/// The half of the original sentence worth keeping is the converse the caller asked for:
/// on a page whose marks are **cached glyph fills** the two settings are one rasteriser
/// under two names, so a page-wide comparison of `Coverage::Cpu` against `Coverage::Gpu`
/// averages marks the setting moved together with marks it could not move.
pub(crate) fn device(coverage: Coverage, atlas: Option<u64>, samples: u32) -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        coverage,
        coverage_samples: samples,
        atlas_budget: atlas.unwrap_or(Options::default().atlas_budget),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this runs")
}

/// One row of the sweep: how the picture is stated, and what the atlas is allowed to do
/// with it. A struct rather than five `bool` parameters, because every one of them
/// changes which lane answers and a reader of a call site should see which is which.
#[derive(Clone, Copy)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "each field is an independent axis of the sweep and every one of them changes \
              which lane answers; named fields at the call site are the point, and are what \
              this struct replaced five positional parameters with"
)]
pub(crate) struct Case {
    /// A rule across the target, rather than down it.
    pub(crate) horizontal: bool,
    /// The position is carried by the command's affine (a PDF's `cm`) rather than by the
    /// outline's own coordinates.
    pub(crate) through_transform: bool,
    /// A second placement of the same outline, so its key is not single-use.
    pub(crate) repeated: bool,
    /// A fill of the rule's own band rather than a stroke of its centre line.
    pub(crate) filled: bool,
    /// An atlas too small to hold the tile, which is what keeps a *fill* off the glyph
    /// lane. It does nothing at all for a stroke — see [`device`].
    pub(crate) uncached: bool,
    /// The rule's device width: [`HAIRLINE`] where the question is placement,
    /// [`WITNESS_WIDTH`] where it is ink.
    pub(crate) width: f32,
}

/// A rule across the target at `centre`, in the axis named by `horizontal`.
///
/// Each call is its own scene *and its own command*, which is the unit the caller's
/// finding is stated in: "the offset is constant within one drawing command and different
/// between commands".
///
/// `through_transform` is the difference between the two ways a host can state the same
/// picture, and it is the whole reason this instrument has a flag: the position can be in
/// the outline's own coordinates, or the outline can sit at the origin and the *command's*
/// affine can carry it — which is what a PDF's `q … cm … S … Q` produces and therefore what
/// the caller's four pages are made of.
pub(crate) fn rule(device: &mut Device, centre: f32, case: Case) -> Scene {
    let placement = placement_of(centre, case);
    if case.filled {
        return band(device, case, placement);
    }
    let mut builder = SceneBuilder::new();
    stroke_into(device, &mut builder, centre, case, placement);
    // **The second placement is the whole of what `repeated` means.** `CacheProspect`
    // answers `worth_caching` on a key used *once* in a frame with `false` (ADR 0065), so a
    // fixture drawing one rule can never reach the atlas — and the atlas is where a phase
    // is quantised. Graph paper is one outline drawn many times, so a decoy placement of
    // the same outline is what makes this fixture the caller's page rather than a hairline
    // on its own. It is drawn far from the measured rule, in the half of the target no
    // profile reads.
    if case.repeated {
        let decoy = if case.horizontal {
            Affine::translate(0.0, -DECOY_GAP)
        } else {
            Affine::translate(-DECOY_GAP, 0.0)
        };
        stroke_into(device, &mut builder, centre, case, placement.then(decoy));
    }
    builder.finish()
}

/// The caller's graph paper: `count` parallel rules in **one scene and `count` commands**,
/// each carrying its own position through its own affine, at the pitch their
/// `bug1743245.pdf` states.
///
/// This is the construction their §31.2 measures and the one a swept single rule cannot
/// be: their finding is that the offset "is constant within one drawing command and
/// different between commands", which is a statement about a *set* of commands and is
/// invisible to a fixture that draws one.
pub(crate) fn graph_paper(
    device: &mut Device,
    case: Case,
    first: f32,
    pitch: f32,
    count: u32,
) -> Scene {
    let mut builder = SceneBuilder::new();
    for index in 0..count {
        let centre = pitch.mul_add(index as f32, first);
        let placement = placement_of(centre, case);
        stroke_into(device, &mut builder, centre, case, placement);
    }
    builder.finish()
}

/// Where this case puts the rule: in the outline's own coordinates, or in the command's
/// affine the way a PDF's `q … cm … S … Q` does.
fn placement_of(centre: f32, case: Case) -> Affine {
    if !case.through_transform {
        Affine::IDENTITY
    } else if case.horizontal {
        Affine::scale(CTM, CTM).then(Affine::translate(0.0, centre))
    } else {
        Affine::scale(CTM, CTM).then(Affine::translate(centre, 0.0))
    }
}

/// How far the rule's centre line runs, in the space the placement scales.
fn span(case: Case) -> f32 {
    let along = f32::from(u16::try_from(ALONG).expect("the target is far below 65 536"));
    (along + 4.0) / if case.through_transform { CTM } else { 1.0 }
}

/// One stroked placement of the rule's centre line into `builder`.
///
/// A **stroke**, because `quorra_scene::axis_aligned_rect` recognises a rectangle's four
/// edges and a solid fill of one takes the analytic rectangle lane, which is exact by
/// construction and is not the lane the question is about
/// (`tests/scale_invariance.rs` states the same reason for the same choice).
fn stroke_into(
    device: &mut Device,
    builder: &mut SceneBuilder,
    centre: f32,
    case: Case,
    placement: Affine,
) {
    // The caller's witness is `0.5-unit strokes under a 0.317 CTM`, so when the position
    // is stated through the affine it is stated the way their page states it: the outline
    // in the user space that CTM scales, not in device pixels.
    let stated = if case.through_transform { 0.0 } else { centre };
    let reach = span(case);
    let (from, to) = if case.horizontal {
        (Point::new(-reach, stated), Point::new(reach, stated))
    } else {
        (Point::new(stated, -reach), Point::new(stated, reach))
    };
    let outline = device
        .upload_outline(&[Segment::MoveTo(from), Segment::LineTo(to)])
        .expect("two points are inside every coordinate ceiling");
    builder
        .stroke(
            outline,
            placement,
            Stroke {
                width: case.width,
                cap: LineCap::Butt,
                join: LineJoin::Miter,
                miter_limit: 10.0,
            },
            Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0)),
            None,
            BlendMode::Normal,
            None,
        )
        .expect("a hairline is inside every scene budget");
}

/// The same rule as a **fill** of its own band, which is the other way a host can state a
/// hairline and the only one that reaches the atlas at all.
///
/// Six points rather than four, and the extra two are collinear: `axis_aligned_rect`
/// accepts `M,L,L,L[,L-back-to-start][,Close]` and refuses anything longer
/// (`geom/segment.rs`), so a sixth point keeps the geometry an exact axis-aligned band
/// while sending it to the coverage lane instead of the analytic rectangle one. The band
/// is the shape; the recogniser is what is being avoided, not the definition.
fn band(device: &mut Device, case: Case, placement: Affine) -> Scene {
    // In the outline's own space, which the placement scales — the band sits at zero and
    // the affine carries it, exactly as the stroke above does. Getting this wrong is how an
    // earlier run of this instrument reported a six-pixel "error" that was a user-space
    // half-width added to a device-space centre.
    let half = case.width / 2.0 / CTM;
    let reach = span(case);
    let point = |along: f32, across: f32| {
        if case.horizontal {
            Point::new(along, across)
        } else {
            Point::new(across, along)
        }
    };
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(point(-reach, -half)),
            Segment::LineTo(point(0.0, -half)),
            Segment::LineTo(point(reach, -half)),
            Segment::LineTo(point(reach, half)),
            Segment::LineTo(point(0.0, half)),
            Segment::LineTo(point(-reach, half)),
            Segment::Close,
        ])
        .expect("six points are inside every coordinate ceiling");
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            placement,
            FillRule::NonZero,
            Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0)),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("a hairline is inside every scene budget");
    builder.finish()
}
