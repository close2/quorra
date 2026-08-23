//! A group's clip is a **set**, and a group whose alpha is provably its shape meets it
//! by intersection rather than by a product (ADR 0074).
//!
//! # Where the expected values come from
//!
//! ISO 32000-2 §11.3.7.2 lists what multiplies, and a clip is in neither of its two
//! products:
//!
//! > The three shape inputs shall be multiplied together, producing an intermediate
//! > value called the source shape.
//!
//! > The three opacity inputs shall be multiplied together, producing an intermediate
//! > value called the source opacity.
//!
//! §8.5.4 puts the clip one step earlier — on the object's own shape — and says it of a
//! group in its own sentence:
//!
//! > In the context of the transparent imaging model (PDF 1.4), the current clipping
//! > path constrains an object’s shape (see 11.2, "Overview of transparency"). The
//! > effective shape is the intersection of the object’s intrinsic shape with the
//! > clipping path; the source shape value shall be 0.0 outside this intersection.
//! > Similarly, the shape of a transparency group (defined as the union of the shapes of
//! > its constituent objects) shall be influenced both by the clipping path in effect
//! > when each of the objects is painted and by the one in effect at the time the
//! > group’s results are painted onto its backdrop.
//!
//! and §10.7.4 makes "influenced by" a set operation rather than an arithmetic one:
//!
//! > For clipping, the clipping region consists of the set of pixels that would be
//! > included by a fill operation. Subsequent painting operations shall affect a region
//! > that is the intersection of the set of pixels defined by the clipping region with
//! > the set of pixels for the region to be painted.
//!
//! The fractions come from §11.3.7.2's NOTE 1, which is why a set operation has
//! fractional values to argue about at all: "when such objects are rasterized to device
//! pixels, the shape values along the boundaries can be anti-aliased, taking on
//! fractional values representing fractional coverage of those pixels".
//!
//! # The arithmetic each test asserts, by hand
//!
//! Every fixture here draws one black rectangle whose right edge falls **inside** device
//! column 2, under a clip whose own edge falls in the same column, and reads the alpha of
//! a pixel in that column. Two coverages are used because both are exactly representable
//! in eight bits, which takes rounding out of the derivation: `0.6 = 153/255` and
//! `0.2 = 51/255`.
//!
//! | fixture | group shape `S` | clip `C` | `S ∩ C` | `S × C` |
//! |---|---|---|---|---|
//! | edges coincide | 0.6 | 0.6 | **0.6** → 153 | 0.36 → 92 |
//! | clip contains the group | 0.2 | 0.6 | **0.2** → 51 | 0.12 → 31 |
//!
//! The intersection column is not an estimate in either row. In the first the two regions
//! are the same half-plane, and a region intersected with itself is the region; in the
//! second the group's half-plane lies inside the clip's, and `S ∩ C = S` where `S ⊆ C`.
//! The product column is what the compositor drew before ADR 0074, and what it still
//! draws where the encoder cannot prove that a group's alpha is its shape — the square of
//! the truth in the first row, and 0.2 × 0.6 in the second.
//!
//! `× 255` and rounding to nearest is §3's straight-alpha readback: 0.36 × 255 = 91.8 and
//! 0.12 × 255 = 30.6, so 92 and 31.
//!
//! # What decides which arithmetic a group gets, and why two fixtures are drawn wrong
//!
//! `encode::opacity::every_opacity_is_one` proves the condition from the group's own
//! commands: no opacity input below 1.0 anywhere inside it. Two tests here draw groups it
//! declines — one whose opacity is genuinely below 1, where the product is **right**, and
//! one carrying a soft mask worth 1.0 at every pixel, where the product is what this
//! backend draws and is **not** the clause's value. The second is the conservative half of
//! ADR 0074 measured rather than described: a mask cannot change what any pixel should be
//! and it changes the route, which is the caller's own probe design in their §36.3.

// Test-file lint policy as in m1.rs; the arithmetic below is the clause's, over rasters
// this file just drew.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Device, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, ClipId, Color, Compose, FillRule, GroupSpec, MaskId, MaskKind, Point, Rect,
    Scene, SceneBuilder, Segment,
};

mod common;

use common::probe::alpha;
use common::scene::rect_outline;

/// The target: wide enough to hold the fixture's column 2 and its neighbours, short
/// enough that every row of it is the same row.
const WIDTH: u32 = 16;
const HEIGHT: u32 = 8;

/// The device column both edges fall inside, and the row every assertion reads.
const EDGE_COLUMN: u32 = 2;
const ROW: u32 = 3;

/// `0.6` of a pixel: the coverage of column 2 under a right edge at x = 2.6, and
/// `153/255` exactly, so nothing here rounds.
const SIX_TENTHS: f32 = 2.6;
/// `0.2` of a pixel, and `51/255` exactly.
const TWO_TENTHS: f32 = 2.2;

/// What a group's content is made of, which is what decides whether the encoder can prove
/// the group's alpha to be its shape.
#[derive(Clone, Copy)]
enum Content {
    /// One opaque rectangle: every opacity input is 1.0, so the raster's alpha is
    /// §11.6.4.2's group shape.
    Opaque,
    /// The same rectangle at half opacity: §11.6.4.4's constant, which the raster carries
    /// multiplied into the same number as the shape.
    HalfOpaque,
    /// The same rectangle under a soft mask worth 1.0 at every pixel: an opacity input
    /// (ADR 0066) whose *value* changes nothing and whose presence the encoder cannot
    /// value, so the group falls back to the product.
    MaskedByOne,
}

fn black(alpha: f32) -> Color {
    Color::new(0.0, 0.0, 0.0, alpha)
}

/// A soft mask worth 1.0 at every pixel of the target: §11.5.2's alpha rule over one
/// opaque rectangle covering the whole of it.
fn mask_of_ones(builder: &mut SceneBuilder) -> MaskId {
    builder
        .mask(MaskKind::Alpha, None, |body| {
            body.rect(
                Rect::new(
                    Point::new(0.0, 0.0),
                    Point::new(WIDTH as f32, HEIGHT as f32),
                ),
                Affine::IDENTITY,
                black(1.0),
                None,
                None,
            )
        })
        .unwrap()
}

/// The mark: one rectangle from the left edge of the target to `right`, over the whole
/// height, so every row of column `EDGE_COLUMN` carries the same coverage.
fn mark(builder: &mut SceneBuilder, right: f32, content: Content) {
    let mask = match content {
        Content::MaskedByOne => Some(mask_of_ones(builder)),
        Content::Opaque | Content::HalfOpaque => None,
    };
    let alpha = match content {
        Content::HalfOpaque => 0.5,
        Content::Opaque | Content::MaskedByOne => 1.0,
    };
    builder
        .rect(
            Rect::new(Point::new(0.0, 0.0), Point::new(right, HEIGHT as f32)),
            Affine::IDENTITY,
            black(alpha),
            None,
            mask,
        )
        .unwrap();
}

/// A rectangular clip whose right edge is `right`: one device rectangle and no residue,
/// so the composite meets it through `clip_coverage` alone (ADR 0007).
fn rect_clip(device: &mut Device, builder: &mut SceneBuilder, right: f32) -> ClipId {
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(-4.0, -4.0),
            Point::new(right, HEIGHT as f32 + 4.0),
        )))
        .unwrap();
    builder
        .clip(outline, Affine::IDENTITY, FillRule::NonZero, None)
        .unwrap()
}

/// The group under test: isolated, opaque, unblended, so that the only thing between the
/// child raster and the target is the clip.
fn group(clip: Option<ClipId>) -> GroupSpec {
    GroupSpec {
        alpha: 1.0,
        blend: BlendMode::Normal,
        clip,
        knockout: false,
        mask: None,
        compose: Compose::SrcOver,
        isolated: true,
    }
}

/// One mark of width `right` and the given content, inside a group clipped by `clip`.
fn grouped(device: &mut Device, right: f32, clip: Option<f32>, content: Content) -> Scene {
    let mut builder = SceneBuilder::new();
    let clip = clip.map(|edge| rect_clip(device, &mut builder, edge));
    builder
        .group(group(clip), |body| {
            mark(body, right, content);
            Ok(())
        })
        .unwrap();
    builder.finish()
}

/// The same mark with no group and no clip: what the edge's own coverage looks like when
/// nothing has been done to it, which is what `S ∩ C = S` claims a containing clip leaves
/// behind.
fn alone(right: f32) -> Scene {
    let mut builder = SceneBuilder::new();
    mark(&mut builder, right, Content::Opaque);
    builder.finish()
}

fn render(device: &mut Device, scene: &Scene) -> Vec<u8> {
    device
        .render(
            scene,
            &Viewport::full(WIDTH, HEIGHT, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders")
        .into_raster()
        .unwrap()
        .into_pixels()
}

/// The alpha of the boundary pixel of a frame this file drew.
fn edge_alpha(pixels: &[u8]) -> u8 {
    alpha(pixels, WIDTH, EDGE_COLUMN, ROW)
}

/// A group's `/BBox`-shaped clip standing on the group's own edge takes nothing from it.
///
/// The common form-XObject shape: the clip in force when the group's results are painted
/// is exactly the rectangle the group's content fills. Both regions are the half-plane
/// `x ≤ 2.6`, so §8.5.4's intersection is that half-plane and the boundary pixel keeps
/// its own 0.6 — **153**, the same byte the mark draws with no group and no clip at all,
/// which is the assertion made here because it needs no arithmetic to read.
///
/// The product reading is 0.6 × 0.6 = 0.36, or 92, so the two are 61 bytes apart on this
/// fixture.
#[test]
fn a_group_whose_clip_stands_on_its_own_edge_paints_that_edge_once() {
    let mut device = common::headless::device();
    let scene = grouped(&mut device, SIX_TENTHS, Some(SIX_TENTHS), Content::Opaque);
    let clipped = render(&mut device, &scene);
    let bare_scene = alone(SIX_TENTHS);
    let bare = render(&mut device, &bare_scene);

    assert_eq!(
        edge_alpha(&clipped),
        153,
        "0.6 of a pixel intersected with the same 0.6 is 0.6, which is 153 of 255"
    );
    assert_eq!(
        clipped, bare,
        "a clip standing on the group's own edge admits the whole group, so the frame is \
         the frame the mark draws unclipped"
    );
}

/// `S ∩ C = S` where `S ⊆ C`: a clip that contains the group takes nothing from it, at
/// every pixel, its anti-aliased boundary included.
///
/// The group's edge is at x = 2.2 and the clip's at x = 2.6, so the two boundaries are
/// **different** and the group's half-plane lies strictly inside the clip's. The exact
/// area of the intersection inside column 2 is therefore 0.2, which is what `min` gives;
/// the product gives 0.12. This is the row of the table where `min` is provably exact
/// rather than merely no further from the truth.
#[test]
fn a_clip_that_contains_the_group_takes_nothing_from_it() {
    let mut device = common::headless::device();
    let scene = grouped(&mut device, TWO_TENTHS, Some(SIX_TENTHS), Content::Opaque);
    let clipped = render(&mut device, &scene);
    let bare_scene = alone(TWO_TENTHS);
    let bare = render(&mut device, &bare_scene);

    assert_eq!(
        edge_alpha(&clipped),
        51,
        "0.2 of a pixel inside a clip admitting 0.6 of it is 0.2, which is 51 of 255"
    );
    assert_eq!(
        clipped, bare,
        "the clip contains the group, so it removes nothing from any pixel of it"
    );
}

/// **The safety property, and the reason the predicate exists**: a group whose opacity is
/// below 1 must *not* be intersected with its clip.
///
/// The content covers column 2 completely at half opacity, so the raster's alpha there is
/// 0.5 — a `q`, not an `f`. §8.5.4 intersects the clip with the group's shape, which is 1,
/// and §11.3.7.1 then multiplies the opacity in: `min(1, 0.6) × 0.5 = 0.3`, which is the
/// **77** below (the layer stores 0.5 as 128/255, so 0.50196 × 0.6 × 255 = 76.8). Taking
/// `min` against the alpha instead would give `min(0.5, 0.6) = 0.5`, or 128 — a
/// half-transparent group painted at more than it asked for, over the whole boundary.
#[test]
fn a_group_whose_opacity_is_below_one_is_not_intersected_with_its_clip() {
    let mut device = common::headless::device();
    let scene = grouped(
        &mut device,
        WIDTH as f32,
        Some(SIX_TENTHS),
        Content::HalfOpaque,
    );
    let pixels = render(&mut device, &scene);

    let read = i16::from(edge_alpha(&pixels));
    assert!(
        (read - 77).abs() <= 1,
        "0.5 of opacity under a clip admitting 0.6 is 0.3 of alpha, which is 77 of 255; \
         read {read}, and 128 would be the clip intersected with an opacity"
    );
}

/// The conservative half, measured: a group carrying an opacity input the encoder cannot
/// value keeps the product, **even where the product is not the clause's value**.
///
/// The mask is 1.0 at every pixel, so under §11.3.7.1 it changes nothing — `f × 1 = f` —
/// and the clause's answer for this fixture is still 153. What it changes is the route:
/// `every_opacity_is_one` sees a soft mask, cannot know it is 1 everywhere without
/// rendering it, and answers `false`, so the clip multiplies and the edge is drawn at
/// **92**.
///
/// This is a hole in the improvement rather than a defect introduced by it — it is what
/// every group got before ADR 0074 — and it is asserted rather than described so that
/// closing it later is a visible change and not a silent one.
#[test]
fn a_group_whose_opacity_cannot_be_valued_keeps_the_product() {
    let mut device = common::headless::device();
    let scene = grouped(
        &mut device,
        SIX_TENTHS,
        Some(SIX_TENTHS),
        Content::MaskedByOne,
    );
    let pixels = render(&mut device, &scene);

    assert_eq!(
        edge_alpha(&pixels),
        92,
        "a mask worth 1.0 everywhere leaves the clause's answer at 153 and takes the \
         product route: 0.6 × 0.6 = 0.36, which is 92 of 255"
    );
}

/// The two routes agree where the clip has no fractional edge to argue about.
///
/// The clip's edge is at x = 8, an integer, so `clip_coverage` is 1 in every column the
/// group marks and 0 beyond — and 1 is the identity of both a product and a minimum. A
/// frame that differed here would mean the intersection had been applied to something
/// other than the clip.
#[test]
fn an_integral_clip_edge_reads_the_same_on_both_routes() {
    let mut device = common::headless::device();
    let proved = grouped(&mut device, SIX_TENTHS, Some(8.0), Content::Opaque);
    let proved = render(&mut device, &proved);
    let declined = grouped(&mut device, SIX_TENTHS, Some(8.0), Content::MaskedByOne);
    let declined = render(&mut device, &declined);

    assert_eq!(edge_alpha(&proved), 153);
    assert_eq!(
        proved, declined,
        "a clip that admits whole pixels is the identity of both routes"
    );
}

/// The two halves of one clip chain — the resolved rectangle and the rasterised residue —
/// intersect at the group's blit as well, which is ADR 0030's rule at a site that had
/// been multiplying them.
///
/// The chain is a rectangle clip at x ≤ 2.6 and a pentagon whose only near edge is the
/// same vertical line, so the two links are one region restated. The group's content fills
/// column 2 completely, which takes the group's own shape out of the question and leaves
/// exactly the chain: `min(0.6, 0.6)` is 0.6 — **153** — where the product of the two
/// links is 0.36, or 92. This holds whatever the group's opacity is provable to be,
/// because it is a statement about the chain and not about the group.
#[test]
fn the_links_of_one_chain_intersect_at_a_groups_blit() {
    let mut device = common::headless::device();
    let mut builder = SceneBuilder::new();
    let rect = rect_clip(&mut device, &mut builder, SIX_TENTHS);
    // A pentagon rather than a rectangle: `axis_aligned_rect` recognises the four-sided
    // form and resolves it into the chain's rectangle, which is the very path this test
    // must not take. The fifth corner is at x = −2, eight columns from the pixel read.
    let pentagon = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(-4.0, -4.0)),
            Segment::LineTo(Point::new(SIX_TENTHS, -4.0)),
            Segment::LineTo(Point::new(SIX_TENTHS, HEIGHT as f32 + 4.0)),
            Segment::LineTo(Point::new(-2.0, HEIGHT as f32 + 4.0)),
            Segment::LineTo(Point::new(-4.0, HEIGHT as f32)),
            Segment::Close,
        ])
        .unwrap();
    let chain = builder
        .clip(pentagon, Affine::IDENTITY, FillRule::NonZero, Some(rect))
        .unwrap();
    builder
        .group(group(Some(chain)), |body| {
            mark(body, WIDTH as f32, Content::Opaque);
            Ok(())
        })
        .unwrap();
    let scene = builder.finish();

    let frame = device
        .render(
            &scene,
            &Viewport::full(WIDTH, HEIGHT, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders");
    let residues = frame.counters().clip_residue_regions;
    let pixels = frame.into_raster().unwrap().into_pixels();

    assert_eq!(
        residues, 1,
        "the pentagon must reach the composite as a residue; a chain resolved entirely \
         into its rectangle would satisfy the assertion below without ever composing two \
         links"
    );
    assert_eq!(
        edge_alpha(&pixels),
        153,
        "one region stated twice admits what it admits once: min(0.6, 0.6) = 0.6, which \
         is 153 of 255, where the product of the links is 92"
    );
}
