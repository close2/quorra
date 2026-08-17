//! `Counters::lanes`: the instrument §11.2's census is taken with
//! (`doc/notes-census.md`), beside ADR 0057's `Counters::coverage`, which is what the
//! census reads for the work a lane causes.
//!
//! §1.1 of `doc/PLAN.md` asserts that most of a page is repeated glyph outlines and
//! axis-aligned rectangles and that general curve filling is the *rare* case. That is the
//! premise the whole architecture is arranged around, and every claim in this file is
//! about the counter that can now say whether a given page agrees with it.
//!
//! Each test names the lane it means, because a fixture that names a lane and lands in
//! another is how three tests in `m45.rs` came to compare one lane with itself
//! (ADR 0047). Two conditions run through all of them:
//!
//! - **the lanes count marks, not commands.** A command that reaches no pixel takes no
//!   lane, and a group draws none of its own.
//! - **the rectangle lane rasterises nothing.** That is what makes it the fast lane and
//!   what §6.4 of the brief insists on, so it is asserted as a zero rather than assumed.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects
)]

use std::sync::Arc;

use quorra_gpu::{Counters, Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Compose, FillRule, ImageFilter, ImageSpec, LineCap, LineJoin, Point, Rect,
    Scene, SceneBuilder, Segment, Stroke,
};

mod common;

use common::scene::{black, rect_outline};

const SIZE: u32 = 64;

/// An atlas small enough that the fills below outgrow it, for the one test that is about
/// a tile the cache will not admit. 4 KiB admits tiles of 512 texels (ADR 0024's eighth
/// share), which a 40 × 40 mark is eighty times over.
const TINY_ATLAS: u64 = 4 * 1024;

fn device() -> Device {
    common::headless::device()
}

fn counters_of(device: &mut Device, scene: &Scene) -> Counters {
    device
        .render(
            scene,
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders")
        .counters()
}

/// A triangle: three edges, so `axis_aligned_rect` refuses it and the analytic lane
/// cannot have it whatever the transform is.
fn triangle() -> Vec<Segment> {
    vec![
        Segment::MoveTo(Point::new(4.0, 4.0)),
        Segment::LineTo(Point::new(40.0, 8.0)),
        Segment::LineTo(Point::new(12.0, 44.0)),
        Segment::Close,
    ]
}

fn fill(builder: &mut SceneBuilder, outline: quorra_scene::OutlineId, transform: Affine) {
    builder
        .fill(
            outline,
            transform,
            FillRule::NonZero,
            black(),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("a solid fill of an uploaded outline");
}

/// ADR 0047's door: a fill whose outline is four axis-aligned edges takes the analytic
/// rectangle lane, and that lane rasterises no coverage at all — which is the property
/// §6.4 of the brief states and the reason the lane exists.
#[test]
fn a_rectangular_outline_takes_the_rectangle_lane_and_rasterises_nothing() {
    let mut device = device();
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(8.0, 8.0),
            Point::new(40.0, 40.0),
        )))
        .expect("upload");
    let mut builder = SceneBuilder::new();
    fill(&mut builder, outline, Affine::IDENTITY);
    let counters = counters_of(&mut device, &builder.finish());

    assert_eq!(counters.lanes.rectangle, 1, "the analytic lane drew it");
    assert_eq!(counters.lanes.glyph, 0);
    assert_eq!(counters.lanes.path, 0);
    assert_eq!(counters.lanes.image, 0);
    assert_eq!(
        counters.coverage,
        quorra_gpu::CoverageSheet::default(),
        "the rectangle lane makes no coverage bytes; that is what makes it the fast lane"
    );
    assert_eq!(counters.tiles, 0);
}

/// The glyph lane counts **placements**, not tiles: a page that draws one letterform
/// many times is exactly the case §1.1 says a document is mostly made of, and the two
/// numbers that say so are this one and `atlas_distinct_keys`.
#[test]
fn every_placement_of_one_shape_is_its_own_glyph_lane_mark() {
    let mut device = device();
    let outline = device.upload_outline(&triangle()).expect("upload");
    let mut builder = SceneBuilder::new();
    // Whole-pixel translations, so every placement shares one atlas key.
    for i in 0..5 {
        fill(
            &mut builder,
            outline,
            Affine::scale(0.2, 0.2).then(Affine::translate(i as f32 * 9.0, 2.0)),
        );
    }
    let counters = counters_of(&mut device, &builder.finish());

    assert_eq!(counters.lanes.glyph, 5, "five placements");
    assert_eq!(counters.atlas_distinct_keys, 1, "of one tile");
    assert_eq!(counters.lanes.path, 0);
    assert_eq!(counters.lanes.rectangle, 0);
}

/// A stroke never reaches the atlas — its expansion is a polygon, not the outline the
/// cache keys on — so every stroke a page states is a path-lane mark. Over the caller's
/// corpus that is **81 %** of the whole path lane (`doc/notes-census.md`), which is why
/// the population is pinned here rather than left to the corpus alone.
#[test]
fn a_stroke_is_a_path_lane_mark() {
    let mut device = device();
    let outline = device.upload_outline(&triangle()).expect("upload");
    let mut builder = SceneBuilder::new();
    builder
        .stroke(
            outline,
            Affine::IDENTITY,
            Stroke {
                width: 2.0,
                cap: LineCap::Butt,
                join: LineJoin::Miter,
                miter_limit: 4.0,
            },
            black(),
            None,
            BlendMode::Normal,
            None,
        )
        .expect("a stroke of an uploaded outline");
    let counters = counters_of(&mut device, &builder.finish());

    assert_eq!(counters.lanes.path, 1);
    assert_eq!(counters.lanes.glyph, 0);
    assert_eq!(counters.tiles, 1, "one tile of the frame's sheet");
    assert!(
        counters.coverage.texels > 0,
        "the path lane rasterises coverage, unlike the rectangle lane"
    );
}

/// An image placement is the image lane whatever its transform is (ADR 0011): the quad
/// carries its own inverse, so nothing about it is a coverage question.
#[test]
fn an_image_placement_is_the_image_lane() {
    let mut device = device();
    let image = device
        .upload_image(&ImageSpec {
            width: 1,
            height: 1,
            data: Arc::from([255_u8, 0, 0, 255].as_slice()),
        })
        .expect("consistent image");
    let mut builder = SceneBuilder::new();
    builder
        .image(
            image,
            Affine::scale(16.0, 16.0),
            1.0,
            ImageFilter::Nearest,
            None,
            BlendMode::Normal,
            None,
        )
        .expect("valid image command");
    let counters = counters_of(&mut device, &builder.finish());

    assert_eq!(counters.lanes.image, 1);
    assert_eq!(counters.lanes.rectangle, 0);
    assert_eq!(counters.lanes.path, 0);
    assert_eq!(counters.coverage.texels, 0, "no residue clip over it");
}

/// The seam §11.2's census exists to size, in one fixture: the *same* fill takes the
/// glyph lane on a device whose atlas will hold its tile and the path lane on one whose
/// atlas will not. Which lane a mark takes is a device-space question, not a property of
/// the scene (§1.1) — and this is the mechanism by which a page's shares move under
/// magnification.
#[test]
fn one_fill_takes_the_glyph_lane_or_the_path_lane_by_what_the_atlas_will_hold() {
    let shape = triangle();
    let mut roomy = device();
    let outline = roomy.upload_outline(&shape).expect("upload");
    let mut builder = SceneBuilder::new();
    fill(&mut builder, outline, Affine::IDENTITY);
    let scene = builder.finish();
    let cached = counters_of(&mut roomy, &scene);
    assert_eq!(cached.lanes.glyph, 1, "the default atlas holds this tile");
    assert_eq!(cached.lanes.path, 0);

    let mut cramped = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        atlas_budget: TINY_ATLAS,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs");
    let outline = cramped.upload_outline(&shape).expect("upload");
    let mut builder = SceneBuilder::new();
    fill(&mut builder, outline, Affine::IDENTITY);
    let uncached = counters_of(&mut cramped, &builder.finish());
    assert_eq!(uncached.lanes.path, 1, "too large for a 4 KiB atlas");
    assert_eq!(uncached.lanes.glyph, 0);
}

/// A clip whose links are not all rectangles takes a rectangle *out* of its lane: the
/// residue has to multiply into coverage bytes, and the analytic lane has nowhere to put
/// one. Over the corpus this is 23 % of the path lane and the second-largest reason a
/// mark is in it.
#[test]
fn a_residue_clip_moves_a_rectangle_into_the_path_lane() {
    let mut device = device();
    let rect = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(8.0, 8.0),
            Point::new(40.0, 40.0),
        )))
        .expect("upload");
    let curved = device.upload_outline(&triangle()).expect("upload");

    let mut builder = SceneBuilder::new();
    let clip = builder
        .clip(curved, Affine::IDENTITY, FillRule::NonZero, None)
        .expect("a clip of an uploaded outline");
    builder
        .fill(
            rect,
            Affine::IDENTITY,
            FillRule::NonZero,
            black(),
            Some(clip),
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("a clipped solid fill");
    let counters = counters_of(&mut device, &builder.finish());

    assert_eq!(counters.lanes.rectangle, 0, "the residue took it out");
    assert_eq!(counters.lanes.path, 1);
    assert_eq!(
        counters.clip_residue_regions + counters.clip_residue_tiles,
        1
    );
}

/// A command that reaches no pixel takes no lane at all, and the counter that accounts
/// for it is `commands_culled`. Without this the lanes would be a count of commands
/// wearing a different name, and a page at 20× magnification — where a viewer hands over
/// a whole page for a window showing a fortieth of it — would report a lane share for
/// marks nobody drew.
#[test]
fn a_command_that_reaches_no_pixel_takes_no_lane() {
    let mut device = device();
    let outline = device.upload_outline(&triangle()).expect("upload");
    let mut builder = SceneBuilder::new();
    fill(&mut builder, outline, Affine::IDENTITY);
    fill(
        &mut builder,
        outline,
        Affine::translate(10_000.0, 10_000.0), // far off the target
    );
    let counters = counters_of(&mut device, &builder.finish());

    assert_eq!(counters.commands, 2);
    assert_eq!(counters.commands_culled, 1);
    let marks = counters.lanes.rectangle
        + counters.lanes.glyph
        + counters.lanes.path
        + counters.lanes.image;
    assert_eq!(marks, 1, "one command drew, one reached no pixel");
}

/// The whole vocabulary at once, so that the four fields are checked to **partition**
/// the marks rather than each being right in isolation: a mark counted in two lanes, or
/// in none, would pass every test above.
#[test]
fn the_four_lanes_partition_a_mixed_page_s_marks() {
    let mut device = device();
    let rect = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(2.0, 2.0),
            Point::new(20.0, 10.0),
        )))
        .expect("upload");
    let curved = device.upload_outline(&triangle()).expect("upload");
    let image = device
        .upload_image(&ImageSpec {
            width: 1,
            height: 1,
            data: Arc::from([0_u8, 255, 0, 255].as_slice()),
        })
        .expect("consistent image");

    let mut builder = SceneBuilder::new();
    fill(&mut builder, rect, Affine::IDENTITY); // rectangle
    fill(&mut builder, curved, Affine::scale(0.3, 0.3)); // glyph
    fill(&mut builder, curved, Affine::translate(0.0, 1.0)); // glyph, second key
    builder
        .stroke(
            curved,
            Affine::IDENTITY,
            Stroke {
                width: 1.0,
                cap: LineCap::Round,
                join: LineJoin::Round,
                miter_limit: 4.0,
            },
            black(),
            None,
            BlendMode::Normal,
            None,
        )
        .expect("a stroke"); // path
    builder
        .image(
            image,
            Affine::scale(8.0, 8.0),
            1.0,
            ImageFilter::Nearest,
            None,
            BlendMode::Normal,
            None,
        )
        .expect("valid image command"); // image
    let counters = counters_of(&mut device, &builder.finish());

    assert_eq!(counters.lanes.rectangle, 1);
    assert_eq!(counters.lanes.glyph, 2);
    assert_eq!(counters.lanes.path, 1);
    assert_eq!(counters.lanes.image, 1);
    let marks = counters.lanes.rectangle
        + counters.lanes.glyph
        + counters.lanes.path
        + counters.lanes.image;
    assert_eq!(marks, counters.commands, "every command drew exactly once");
}
