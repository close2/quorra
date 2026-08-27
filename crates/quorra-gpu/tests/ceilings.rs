//! What a document-derived number does when it reaches the top of its range: a value,
//! never an abort, and never a plausible-looking wrong page.
//!
//! `doc/notes-ceilings-audit.md` is the round this file witnesses. The question it answers
//! is the caller's, from `pdf-viewer/doc/HAYRO_ISSUES_FOR_QUORRA.md` §1:
//!
//! > If quorra has an equivalent ceiling anywhere in strip generation, the thing to check
//! > is not whether it can be raised but whether crossing it returns rather than aborts.
//!
//! **What is here and what is elsewhere.** A fixture with two copies is two fixtures, so
//! this file holds only the ceilings nothing else gates. The ones that already have a
//! witness are named here once so that the enumeration can be read in one place:
//! `m1.rs` crosses the frame budget, the adapter's target limit and a non-finite viewport
//! transform; `m2.rs` and `resources.rs`'s own tests cross the resource budget, the
//! outline coordinate bound and the identifier space; `tiling_ceiling.rs` crosses the
//! coverage sheet's height; `m8.rs` crosses the damage list's validity;
//! `quorra-scene`'s `validate.rs` and `frames.rs` cross every scene-boundary refusal and
//! the group-depth bound.
//!
//! What this file adds is the two things that had none:
//!
//! 1. **The third factor of a device coordinate.** A scene point and a command transform
//!    are both bounded by `MAX_COORDINATE`; the viewport transform was checked for
//!    finiteness alone, so the product could leave `f32` — and an infinity does not stop
//!    a frame, it paints a coverage row solid. It is now a refusal that names the limit.
//! 2. **The two ends of the range that is left.** With all three factors bounded, a
//!    device coordinate reaches about `2e27` and a device *delta* can be anything down to
//!    the float grid. Both ends were arithmetic silences in the rasteriser, and both are
//!    crossed here through the public API rather than argued about in a comment.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Device, Options, RenderError, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, LineCap, LineJoin, MAX_COORDINATE, Paint, Point,
    Scene, SceneBuilder, Segment, Stroke,
};

mod common;

use common::probe::alpha;

const SIZE: u32 = 64;

fn device() -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

/// The straight-alpha bytes of `scene` drawn into a `SIZE` × `SIZE` readback target under
/// `transform`.
fn render(device: &mut Device, scene: &Scene, transform: Affine) -> Vec<u8> {
    device
        .render(
            scene,
            &Viewport::full(SIZE, SIZE, transform),
            Target::Readback,
        )
        .expect("the frame draws")
        .into_raster()
        .unwrap()
        .into_pixels()
}

fn black() -> Paint {
    Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0))
}

/// **A viewport transform above the bound the scene boundary already holds is refused,
/// by name, before anything is encoded.**
///
/// The scene boundary refuses a rectangle corner or a command transform coefficient above
/// `MAX_COORDINATE` (`quorra-scene`'s `validate.rs`), and an outline point above it
/// (`ResourceProblem::OutlineCoordinateTooLarge`). The viewport transform is the third
/// factor of every device coordinate and was checked for finiteness alone, so
/// `point × command × viewport` could reach infinity while every input was legal.
///
/// **An infinity is not a stopped frame.** `raster::fill_mask`'s prefix sum carries a NaN
/// to the end of its row, and the non-zero rule's `abs().min(1.0)` returns 1.0 for a NaN
/// — so the frame that came back was a solid band, drawn, reported as drawn. That is §5's
/// third state, which is why this bound is a refusal rather than a clamp.
#[test]
fn a_viewport_transform_above_the_coordinate_bound_is_refused_by_name() {
    let mut device = device();
    let scene = SceneBuilder::new().finish();
    let over = Affine::scale(MAX_COORDINATE * 2.0, 1.0);
    match device.render(&scene, &Viewport::full(SIZE, SIZE, over), Target::Readback) {
        Err(RenderError::ViewportTransformTooLarge { coefficient, limit }) => {
            // Exact, and the comparison is exact on purpose: both numbers are the ones
            // the caller and the constant handed in, copied rather than computed, so a
            // difference of any size is a refusal reporting something else.
            #[allow(clippy::float_cmp)]
            {
                assert_eq!(limit, MAX_COORDINATE, "the refusal names the stated bound");
                assert_eq!(coefficient, MAX_COORDINATE * 2.0, "and what crossed it");
            }
        }
        other => panic!("expected ViewportTransformTooLarge, got {other:?}"),
    }
}

/// The bound itself is legal: a refusal one step past a limit must not also refuse the
/// limit. Crossing a ceiling is a value, and *not* crossing it is a frame.
#[test]
fn a_viewport_transform_at_the_coordinate_bound_still_draws() {
    let mut device = device();
    let mut builder = SceneBuilder::new();
    // One rectangle whose device box is the middle of the page once the viewport's
    // `MAX_COORDINATE` scale has been applied to it.
    let unit = 1.0 / MAX_COORDINATE;
    builder
        .rect(
            quorra_scene::Rect::new(
                Point::new(16.0 * unit, 16.0 * unit),
                Point::new(48.0 * unit, 48.0 * unit),
            ),
            Affine::IDENTITY,
            Color::new(0.0, 0.0, 0.0, 1.0),
            None,
            None,
        )
        .unwrap();
    let pixels = render(
        &mut device,
        &builder.finish(),
        Affine::scale(MAX_COORDINATE, MAX_COORDINATE),
    );
    assert_eq!(alpha(&pixels, SIZE, 32, 32), 255, "the rectangle is drawn");
    assert_eq!(alpha(&pixels, SIZE, 2, 2), 0, "and the corner is not");
}

/// **A stroke across the whole coordinate range draws its band**, rather than being drawn
/// as nothing.
///
/// The path below sits at the top of what the contract admits, and every factor is one the
/// boundary checks: the outline's far point is `MAX_COORDINATE`, the command transform
/// scales by `MAX_COORDINATE`, and so does the viewport — so the device delta is `1e27`.
/// `raster::direction` computed a length as `(dx*dx + dy*dy).sqrt()`, and `dx * dx`
/// overflows to infinity above `1.9e19` — eight orders of magnitude below the contract's
/// own ceiling. The length was then infinite, the normal `(0, 0)`, and the stroke's quad
/// had no width: a mark asked for and drawn as nothing, which §5 calls worse than a
/// refusal.
#[test]
fn a_stroke_across_the_coordinate_range_still_draws_its_band() {
    let mut device = device();
    // The composed transform scales x by `MAX_COORDINATE²` and leaves y at 1, so the
    // outline's `MAX_COORDINATE` reaches `1e27` device pixels across while its y stays on
    // the page.
    let command = Affine {
        a: MAX_COORDINATE,
        b: 0.0,
        c: 0.0,
        d: 1.0 / MAX_COORDINATE,
        e: 0.0,
        f: 0.0,
    };
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.0, 32.0)),
            Segment::LineTo(Point::new(MAX_COORDINATE, 32.0)),
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    builder
        .stroke(
            outline,
            command,
            Stroke {
                width: 4.0,
                adjust: false,
                cap: LineCap::Butt,
                join: LineJoin::Miter,
                miter_limit: 10.0,
            },
            black(),
            None,
            BlendMode::Normal,
            None,
        )
        .unwrap();
    let pixels = render(
        &mut device,
        &builder.finish(),
        Affine::scale(MAX_COORDINATE, MAX_COORDINATE),
    );
    assert_eq!(
        alpha(&pixels, SIZE, 32, 32),
        255,
        "the band's own row is inked"
    );
    assert_eq!(
        alpha(&pixels, SIZE, 8, 31),
        255,
        "and it runs across the page"
    );
    assert_eq!(
        alpha(&pixels, SIZE, 32, 8),
        0,
        "the rows away from it are not"
    );
}

/// **A mark thinner than the float grid paints nothing, not a solid row.**
///
/// The triangle is `1e-30` of a device pixel tall and `1e9` wide; both numbers are inside
/// `MAX_COORDINATE`, so a document may state them and the boundary admits them. Its slope
/// is `1e39`, which is infinite in `f32`, and the slab arithmetic then deposited a NaN
/// that the prefix sum carried across the row — where the non-zero rule reads a NaN as
/// full coverage. The frame came back with a black line across the top of the page,
/// reported as drawn.
///
/// Zero is the exact answer rather than a compromise: the area such a mark covers is under
/// `1e-30` where one coverage step is `1/255`.
#[test]
fn a_mark_thinner_than_the_float_grid_paints_no_row() {
    let mut device = device();
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.0, 1e-30)),
            Segment::LineTo(Point::new(MAX_COORDINATE, 2e-30)),
            Segment::LineTo(Point::new(0.0, 2e-30)),
            Segment::Close,
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            black(),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    let pixels = render(&mut device, &builder.finish(), Affine::IDENTITY);
    assert!(
        pixels.iter().skip(3).step_by(4).all(|&a| a == 0),
        "a mark 1e-30 of a pixel tall inks no pixel of the page"
    );
}

/// **A stroke whose segment is shorter than the float grid still draws the mark the clause
/// gives it**, rather than nothing at all.
///
/// `stroke_polylines` drops coincident neighbours by comparing coordinates for equality,
/// and two points `1e-30` apart are not equal — while `dx * dx` for that delta underflows
/// to zero, so the length was zero and the direction `(NaN, ±inf)`. CLAUDE.md's rule is
/// that a document's numbers must "never produce NaN geometry", and this was the one place
/// in the tree that did; `raster.rs`'s
/// `a_segment_below_the_float_grid_produces_finite_geometry` holds that invariant on the
/// expansion itself, and this holds the picture it leads to.
///
/// **The picture is a disc**, and it comes from ISO 32000-2 §8.4.3.3's Table 53: a round
/// cap is
///
/// > [a] semicircular arc with a diameter equal to the line width […] drawn around the
/// > endpoint
///
/// and §8.4.3.3 applies a cap to *both* ends of an open subpath — so a subpath whose two
/// ends are the same point to within `1e-30` of a device pixel is two opposite semicircles,
/// which is one disc of the line's own width. A `NaN` direction gives `atan2` nothing to
/// work with and the fan collapses, so the mark disappears entirely.
#[test]
fn a_stroke_segment_below_the_float_grid_still_draws_its_cap() {
    let mut device = device();
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.0, 32.0)),
            Segment::LineTo(Point::new(1e-30, 32.0)),
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    builder
        .stroke(
            outline,
            Affine::IDENTITY,
            Stroke {
                width: 8.0,
                adjust: false,
                cap: LineCap::Round,
                join: LineJoin::Miter,
                miter_limit: 10.0,
            },
            black(),
            None,
            BlendMode::Normal,
            None,
        )
        .unwrap();
    let pixels = render(&mut device, &builder.finish(), Affine::IDENTITY);
    // Probe pixels whose *whole cell* is inside the disc, so the answer does not depend on
    // the arc's chords: at radius 4 a chord of `ARC_STEP` cuts 0.06 px inside the circle,
    // and each cell below is more than half a pixel clear of it.
    assert_eq!(
        alpha(&pixels, SIZE, 0, 32),
        255,
        "the disc's own centre is inked"
    );
    assert_eq!(
        alpha(&pixels, SIZE, 2, 32),
        255,
        "and a cell 2.6 px along its radius"
    );
    assert_eq!(alpha(&pixels, SIZE, 0, 30), 255, "and a cell 1.6 px up it");
    assert_eq!(
        alpha(&pixels, SIZE, 0, 22),
        0,
        "while 9.5 px away is outside a radius of 4"
    );
}

/// A frame's first mark begins its batch at instance 0, which is the invariant
/// `Encoder::note_batch`'s `- 1` rests on: it is called only after a whole instance has
/// been written, so the index of the last instance is never below zero.
///
/// In release that subtraction wraps rather than panicking — the caller's reading of
/// hayro #646 is exactly this shape — and a batch that named instance `u32::MAX` would
/// draw nothing at all. A page with one mark on it is the smallest input that would show
/// it.
#[test]
fn no_batch_is_noted_before_its_instance_is_written() {
    let mut device = device();
    let mut builder = SceneBuilder::new();
    builder
        .rect(
            quorra_scene::Rect::new(Point::new(8.0, 8.0), Point::new(24.0, 24.0)),
            Affine::IDENTITY,
            Color::new(0.0, 0.0, 0.0, 1.0),
            None,
            None,
        )
        .unwrap();
    let pixels = render(&mut device, &builder.finish(), Affine::IDENTITY);
    assert_eq!(
        alpha(&pixels, SIZE, 16, 16),
        255,
        "the only mark on the page draws"
    );
}
