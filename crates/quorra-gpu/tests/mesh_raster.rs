//! A mesh reaches the target as the raster it already is (`doc/PLAN.md` integration
//! note 5).
//!
//! # Why this is a gate and not a paragraph
//!
//! ISO 32000-2 §8.7.4.5.5 to §8.7.4.5.8 define four mesh shadings — free-form and
//! lattice-form Gouraud triangle meshes, Coons patch meshes and tensor-product patch
//! meshes — and no rasteriser in either tree has the primitive. The caller rasterises them
//! once, upstream, and shares the result between both of its backends so that a second copy
//! cannot drift; integration note 5 records that we inherit it:
//!
//! > Both of the caller's backends share the pre-rasterised mesh because neither rasteriser
//! > has the primitive and a second copy would drift. We inherit that: we consume the mesh,
//! > we do not re-triangulate it.
//!
//! The caller's hayro reading list reaches the same subject from the other side: their #3
//! is Coons and tensor patches tessellated at a **fixed** grid, with the tessellation not
//! adapting to the resolution and the triangles conflating at their shared edges. Both
//! consequences are decided upstream for us — which is exactly why our side needs a gate
//! rather than an argument. The promise is narrow and total: **the samples that were
//! uploaded are the samples that are drawn, at the device pixels the upload named**. Any
//! filtering, any interpolation, any second tessellation on this side would show up as a
//! colour the uploaded raster does not contain.
//!
//! # What each test pins
//!
//! - the raster is reproduced **texel for texel** at `left + i, top + j`;
//! - its edges are hard: the pixel past the last column is untouched;
//! - it does not move or scale with the **viewport**, which is note 5's stated cost — a
//!   `MeshRaster` is built at device resolution for one placement, so a zoom re-uploads it;
//! - it does not move with the **mark's own transform** either, for §8.7.4.1's reason;
//! - **no colour appears that the upload did not contain**, which is what "we do not
//!   re-triangulate and do not resample" reduces to in pixels;
//! - the samples' own alpha survives, because it is the upstream rasteriser's triangle
//!   coverage and §11.3.7.2 makes that shape.
//!
//! `tests/m7.rs`'s `mesh_samples_at_absolute_device_pixels` is the placement claim in one
//! assertion; this file is the rest of note 5.

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

use std::collections::BTreeSet;
use std::sync::Arc;

use quorra_gpu::{Counters, Device, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Compose, FillRule, ImageSpec, MeshId, MeshSpec, Paint, Point, Rect, Scene,
    SceneBuilder, Segment,
};

mod common;

use common::headless::device;
use common::probe::pixel;
use common::scene::rect_outline;

/// The target every frame here is drawn into. 64 × 4 = 256 bytes a row, the buffer-copy
/// alignment exactly.
const SIZE: u32 = 64;

/// Where the mesh is anchored, in device pixels. Neither coordinate is zero and neither is
/// the same as the other, so a transposed or dropped anchor is a visible failure rather than
/// an invisible one.
const ANCHOR: (i32, i32) = (10, 6);

/// The mesh's extent in texels.
const MESH: u32 = 8;

/// A mesh raster whose every texel is distinguishable from every other.
///
/// `r` carries the column and `g` the row, so a resample, a transpose or a half-texel
/// offset each produce a colour that is not the one the assertion names — and a filtered
/// read produces one that is in the raster's range but at the wrong place, which the
/// texel-for-texel comparison still catches.
fn numbered_texel(x: u32, y: u32) -> [u8; 4] {
    [(x * 31) as u8, (y * 29) as u8, 200, 255]
}

fn spec_from(width: u32, height: u32, texels: &[[u8; 4]]) -> MeshSpec {
    assert_eq!(texels.len(), (width * height) as usize);
    let mut data = Vec::with_capacity(texels.len() * 4);
    for texel in texels {
        data.extend_from_slice(texel);
    }
    MeshSpec {
        left: ANCHOR.0,
        top: ANCHOR.1,
        image: ImageSpec {
            width,
            height,
            data: Arc::from(data.as_slice()),
        },
    }
}

fn numbered_mesh(device: &mut Device) -> MeshId {
    let texels: Vec<[u8; 4]> = (0..MESH)
        .flat_map(|y| (0..MESH).map(move |x| numbered_texel(x, y)))
        .collect();
    device
        .upload_mesh(&spec_from(MESH, MESH, &texels))
        .expect("a consistent mesh")
}

/// A fill of the whole target under `transform`, painted by `mesh`.
///
/// The outline is divided by the transform's scale so that the *device* mark is the same
/// whole target in every case — which is what lets a viewport scale and a command transform
/// be varied while the coverage stays constant.
fn covering_fill(device: &mut Device, mesh: MeshId, scale: f32) -> Scene {
    let side = SIZE as f32 / scale;
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(side, side),
        )))
        .expect("upload");
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Mesh(mesh),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("a valid mesh fill");
    builder.finish()
}

fn render_at(device: &mut Device, scene: &Scene, scale: f32) -> (Vec<u8>, Counters) {
    let frame = device
        .render(
            scene,
            &Viewport::full(SIZE, SIZE, Affine::scale(scale, scale)),
            Target::Readback,
        )
        .expect("the frame is inside every budget");
    let counters = frame.counters();
    (frame.into_raster().unwrap().into_pixels(), counters)
}

/// Assert the whole raster, texel for texel, at the anchor the upload named.
fn assert_the_raster_is_reproduced(pixels: &[u8], what: &str) {
    for y in 0..MESH {
        for x in 0..MESH {
            let at = (ANCHOR.0 as u32 + x, ANCHOR.1 as u32 + y);
            assert_eq!(
                pixel(pixels, SIZE, at.0, at.1),
                numbered_texel(x, y),
                "{what}: texel ({x}, {y}) is not what was uploaded, at device {at:?}"
            );
        }
    }
}

/// Note 5's whole promise in one assertion: what was uploaded is what is drawn, at the
/// device pixels the upload named, with nothing in between.
#[test]
fn the_uploaded_samples_are_the_samples_drawn() {
    let mut device = device();
    let mesh = numbered_mesh(&mut device);
    let scene = covering_fill(&mut device, mesh, 1.0);
    let (pixels, counters) = render_at(&mut device, &scene, 1.0);
    assert_eq!(
        counters.tiles, 0,
        "a rect-hinted mesh fill is analytic — this fixture no longer means the lane it \
         names"
    );
    assert_the_raster_is_reproduced(&pixels, "at scale 1");
}

/// The raster's edges are the raster's: one pixel past the last column and one before the
/// first are untouched, however much coverage the mark has there.
///
/// The absence is the claim, so the presence beside it is the control — without the second
/// pair of assertions a mesh that drew nothing at all would pass.
#[test]
fn the_raster_has_hard_edges_at_its_own_extent() {
    let mut device = device();
    let mesh = numbered_mesh(&mut device);
    let scene = covering_fill(&mut device, mesh, 1.0);
    let (pixels, _) = render_at(&mut device, &scene, 1.0);

    let (left, top) = (ANCHOR.0 as u32, ANCHOR.1 as u32);
    assert_eq!(
        pixel(&pixels, SIZE, left, top),
        numbered_texel(0, 0),
        "the control: the first texel is drawn"
    );
    assert_eq!(
        pixel(&pixels, SIZE, left + MESH - 1, top + MESH - 1),
        numbered_texel(MESH - 1, MESH - 1),
        "the control: the last texel is drawn"
    );
    assert_eq!(
        pixel(&pixels, SIZE, left - 1, top)[3],
        0,
        "one pixel left of the raster is unpainted"
    );
    assert_eq!(
        pixel(&pixels, SIZE, left + MESH, top)[3],
        0,
        "one pixel right of the raster is unpainted"
    );
    assert_eq!(
        pixel(&pixels, SIZE, left, top - 1)[3],
        0,
        "one pixel above the raster is unpainted"
    );
    assert_eq!(
        pixel(&pixels, SIZE, left, top + MESH)[3],
        0,
        "one pixel below the raster is unpainted"
    );
}

/// Note 5's stated cost, drawn: a `MeshRaster` is built at **device resolution** for a
/// placement, so it is anchored to device pixels and a viewport scale does not stretch it.
///
/// This is the assertion that says a zoom re-uploads its meshes rather than magnifying
/// them, and it is the one a reader is most likely to expect the other way round.
#[test]
fn a_viewport_scale_does_not_stretch_the_raster() {
    let mut device = device();
    let mesh = numbered_mesh(&mut device);
    for scale in [1.0_f32, 2.0, 4.0] {
        let scene = covering_fill(&mut device, mesh, scale);
        let (pixels, _) = render_at(&mut device, &scene, scale);
        assert_the_raster_is_reproduced(&pixels, &format!("at viewport scale {scale}"));
    }
}

/// §8.7.4.1's "the geometry of the gradient fill is independent of that of the object being
/// painted", for the one paint that carries no matrix at all: a mesh is anchored to the
/// page, so the mark's transform moves the coverage and nothing else.
#[test]
fn a_command_transform_moves_the_mark_and_not_the_raster() {
    let mut device = device();
    let mesh = numbered_mesh(&mut device);
    // The same device coverage as `covering_fill`, reached by a quarter-size outline under
    // a scale of four rather than by a full-size one under the identity. Every coordinate
    // quarters and quadruples exactly in binary floating point.
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(SIZE as f32 / 4.0, SIZE as f32 / 4.0),
        )))
        .expect("upload");
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::scale(4.0, 4.0),
            FillRule::NonZero,
            Paint::Mesh(mesh),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("a valid mesh fill");
    let (pixels, _) = render_at(&mut device, &builder.finish(), 1.0);
    assert_the_raster_is_reproduced(&pixels, "under a command transform of four");
}

/// The rasterised branch as well as the analytic one: a triangle is not a rectangle, so its
/// coverage reaches the scratch sheet, and the quad that samples the mesh is then the tile
/// rather than the shape. The anchor must survive that change of placement arithmetic.
#[test]
fn a_rasterised_mark_reads_the_raster_at_the_same_device_pixels() {
    let mut device = device();
    let mesh = numbered_mesh(&mut device);
    // A triangle containing the whole mesh: (0, 0) to (SIZE, 0) to (0, SIZE) covers every
    // point with x + y < SIZE, and the mesh's far corner is at (18, 14).
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(SIZE as f32, 0.0)),
            Segment::LineTo(Point::new(0.0, SIZE as f32)),
            Segment::Close,
        ])
        .expect("upload");
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Mesh(mesh),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("a valid mesh fill");
    let (pixels, counters) = render_at(&mut device, &builder.finish(), 1.0);
    assert_eq!(
        counters.tiles, 1,
        "a triangle's coverage has to be rasterised — this fixture no longer means the \
         lane it names"
    );
    assert_the_raster_is_reproduced(&pixels, "under a rasterised mark");
}

/// "We do not re-triangulate", stated in the only terms pixels have: **every colour drawn
/// is a colour that was uploaded**.
///
/// A two-colour checkerboard has no intermediate value anywhere in it. Any filtering
/// between texels, any second tessellation with interpolated vertex colours, and any
/// resampling to a different grid would put one on the target — and would do so *without*
/// moving the edges, which is why this is a distinct test from the texel comparison above.
#[test]
fn no_colour_appears_that_the_upload_did_not_contain() {
    let mut device = device();
    let dark = [20_u8, 20, 20, 255];
    let light = [230_u8, 230, 230, 255];
    let texels: Vec<[u8; 4]> = (0..MESH)
        .flat_map(|y| (0..MESH).map(move |x| if (x + y) % 2 == 0 { dark } else { light }))
        .collect();
    let mesh = device
        .upload_mesh(&spec_from(MESH, MESH, &texels))
        .expect("a consistent mesh");
    let scene = covering_fill(&mut device, mesh, 1.0);
    let (pixels, _) = render_at(&mut device, &scene, 1.0);

    let mut seen = BTreeSet::new();
    for y in 0..MESH {
        for x in 0..MESH {
            seen.insert(pixel(
                &pixels,
                SIZE,
                ANCHOR.0 as u32 + x,
                ANCHOR.1 as u32 + y,
            ));
        }
    }
    assert_eq!(
        seen,
        BTreeSet::from([dark, light]),
        "the raster holds two colours and the target must hold the same two; anything \
         between them is an interpolation this side introduced"
    );
}

/// A mesh sample's own alpha is the upstream rasteriser's triangle coverage, and
/// §11.3.7.2's NOTE 1 makes fractional coverage **shape**:
///
/// > when such objects are rasterized to device pixels, the shape values along the
/// > boundaries can be anti-aliased, taking on fractional values representing fractional
/// > coverage of those pixels. When such anti-aliasing is performed, it is important to
/// > treat the fractional coverage as shape rather than opacity.
///
/// So a partly covered sample has to reach the target as a partly covered sample, with its
/// colour intact — not flattened to opaque, and not multiplied into the colour. The tolerance
/// is one unorm each way: the frame premultiplies on the way in and the readback divides on
/// the way out (§3's straight-alpha boundary), which is two roundings and no more.
#[test]
fn a_samples_own_alpha_reaches_the_target_as_shape() {
    let mut device = device();
    let partial = [40_u8, 160, 240, 128];
    let texels = vec![partial; (MESH * MESH) as usize];
    let mesh = device
        .upload_mesh(&spec_from(MESH, MESH, &texels))
        .expect("a consistent mesh");
    let scene = covering_fill(&mut device, mesh, 1.0);
    let (pixels, _) = render_at(&mut device, &scene, 1.0);

    let probe = pixel(&pixels, SIZE, ANCHOR.0 as u32 + 4, ANCHOR.1 as u32 + 4);
    for channel in 0..4 {
        assert!(
            i32::from(probe[channel]).abs_diff(i32::from(partial[channel])) <= 1,
            "channel {channel}: {probe:?} against the uploaded {partial:?}"
        );
    }
}
