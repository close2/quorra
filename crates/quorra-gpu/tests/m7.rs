//! The M7 harness: the rare-case lanes — images (ISO 32000-2 §8.9.5), axial and
//! radial shadings (§8.7.4.5.2/.3) and pre-rasterised meshes — each held against an
//! expectation derived from the clause, never from another renderer (CLAUDE.md
//! principle 5). ADR 0011 carries the design under test.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects,
    clippy::needless_pass_by_value,
    clippy::too_many_lines
)]

use std::sync::Arc;

use quorra_gpu::{Device, Options, RenderError, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, ImageFilter, ImageId, ImageSpec, MeshId, MeshSpec,
    Paint, Point, RampId, Rect, SceneBuilder, Segment, ShadingKind, Stop,
};

mod common;

use common::headless::{device, render};
use common::scene::rect_outline;

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

fn rgba(width: u32, height: u32, texels: &[[u8; 4]]) -> ImageSpec {
    assert_eq!(texels.len(), (width * height) as usize);
    let mut data = Vec::with_capacity(texels.len() * 4);
    for texel in texels {
        data.extend_from_slice(texel);
    }
    ImageSpec {
        width,
        height,
        data: Arc::from(data.as_slice()),
    }
}

fn grey_ramp() -> Vec<Stop> {
    vec![
        Stop {
            offset: 0.0,
            color: Color::new(0.0, 0.0, 0.0, 1.0),
        },
        Stop {
            offset: 1.0,
            color: Color::new(1.0, 1.0, 1.0, 1.0),
        },
    ]
}

/// §8.9.5 with nearest filtering (`/Interpolate` false, §8.9.5.3's default): a 2×2
/// image magnified ×4 is four exact 4×4 blocks — `textureLoad` of the texel whose
/// cell contains the pixel centre, no sampler arithmetic, every byte derivable from
/// the mapping alone. The identity transform carries no flip, so the image's top
/// row (first in the data, top row at unit y = 1) lands at the **larger** device y.
#[test]
fn nearest_magnification_is_exact_blocks() {
    let mut device = device();
    let top_left = [255, 0, 0, 255];
    let top_right = [0, 255, 0, 255];
    let bottom_left = [0, 0, 255, 255];
    let bottom_right = [255, 255, 0, 255];
    let image = device
        .upload_image(&rgba(
            2,
            2,
            &[top_left, top_right, bottom_left, bottom_right],
        ))
        .expect("consistent image");

    let mut builder = SceneBuilder::new();
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
        .expect("valid image command");
    let pixels = render(&mut device, &builder.finish(), 8, 8);

    for y in 0..8 {
        for x in 0..8 {
            // Pixel centre → unit square → §8.9.5 orientation: sampling row
            // `floor((1 − v)·height)` puts the data's first (top) row at unit
            // y = 1, which under a flip-free transform is the larger device y.
            let u = (x as f32 + 0.5) / 8.0;
            let v = (y as f32 + 0.5) / 8.0;
            let col = usize::from(u >= 0.5);
            let row = usize::from((1.0 - v) >= 0.5); // index into the data's rows
            let expected = match (row, col) {
                (0, 0) => top_left,
                (0, 1) => top_right,
                (1, 0) => bottom_left,
                (1, 1) => bottom_right,
                _ => unreachable!(),
            };
            assert_eq!(
                pixel(&pixels, 8, x, y),
                expected,
                "pixel ({x}, {y}) must be the exact texel"
            );
        }
    }
}

/// §11.6.4.3's constant alpha and the image's own alpha are both opacity: an opaque
/// white 1×1 image at CA 0.5 reads back as white at α ≈ 128, and an image texel at
/// α 128 under CA 1 reads the same.
#[test]
fn constant_alpha_and_image_alpha_are_opacity() {
    let mut device = device();
    let opaque = device
        .upload_image(&rgba(1, 1, &[[255, 255, 255, 255]]))
        .expect("upload");
    let translucent = device
        .upload_image(&rgba(1, 1, &[[255, 255, 255, 128]]))
        .expect("upload");

    for (image, constant) in [(opaque, 0.5_f32), (translucent, 1.0)] {
        let mut builder = SceneBuilder::new();
        builder
            .image(
                image,
                Affine::scale(4.0, 4.0),
                constant,
                ImageFilter::Nearest,
                None,
                BlendMode::Normal,
                None,
            )
            .expect("valid image command");
        let pixels = render(&mut device, &builder.finish(), 4, 4);
        let got = pixel(&pixels, 4, 2, 2);
        assert!(
            (i32::from(got[3]) - 128).abs() <= 1,
            "alpha must be ≈128, got {got:?}"
        );
        assert!(
            got[0] >= 253 && got[1] >= 253 && got[2] >= 253,
            "colour stays white under an alpha that is opacity, got {got:?}"
        );
    }
}

/// Linear filtering (`/Interpolate` true) interpolates between texels through the
/// hardware sampler. Its precision is the driver's (ADR 0011 states the variance),
/// so the gate is shape, not bytes: clamped ends, values strictly between at the
/// midpoint, and monotonic growth along the gradient.
#[test]
fn linear_filtering_interpolates_between_texels() {
    let mut device = device();
    let image = device
        .upload_image(&rgba(2, 1, &[[0, 0, 0, 255], [255, 255, 255, 255]]))
        .expect("upload");
    let mut builder = SceneBuilder::new();
    builder
        .image(
            image,
            Affine::scale(16.0, 4.0),
            1.0,
            ImageFilter::Linear,
            None,
            BlendMode::Normal,
            None,
        )
        .expect("valid image command");
    let pixels = render(&mut device, &builder.finish(), 16, 4);

    let row: Vec<u8> = (0..16).map(|x| pixel(&pixels, 16, x, 2)[0]).collect();
    assert!(row[0] <= 5, "left end clamps to the first texel: {row:?}");
    assert!(
        row[15] >= 250,
        "right end clamps to the last texel: {row:?}"
    );
    assert!(
        row[7] > 60 && row[8] < 195,
        "midpoint pixels interpolate: {row:?}"
    );
    for x in 1..16 {
        assert!(
            i32::from(row[x]) >= i32::from(row[x - 1]) - 2,
            "the gradient is monotonic within tolerance: {row:?}"
        );
    }
}

/// ADR 0007's clip-by-intersection applies to the image lane too: the clipped-away
/// half draws nothing, the kept half draws, and the clip edge antialiases through
/// the same analytic extent as every lane.
#[test]
fn image_respects_rectangular_clips() {
    let mut device = device();
    let image = device
        .upload_image(&rgba(1, 1, &[[0, 255, 0, 255]]))
        .expect("upload");
    let clip_outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(4.0, 8.0),
        )))
        .expect("upload");
    let mut builder = SceneBuilder::new();
    let clip = builder
        .clip(clip_outline, Affine::IDENTITY, FillRule::NonZero, None)
        .expect("valid clip");
    builder
        .image(
            image,
            Affine::scale(8.0, 8.0),
            1.0,
            ImageFilter::Nearest,
            Some(clip),
            BlendMode::Normal,
            None,
        )
        .expect("valid image command");
    let pixels = render(&mut device, &builder.finish(), 8, 8);
    assert_eq!(pixel(&pixels, 8, 2, 4), [0, 255, 0, 255]);
    assert_eq!(pixel(&pixels, 8, 6, 4)[3], 0, "clipped half draws nothing");
}

/// An oblique placement paints exactly the fragments whose centres map inside the
/// unit square — the diamond of a 45° rotation, hard-edged by ADR 0011's stated
/// decision, with nothing outside the footprint.
#[test]
fn oblique_image_paints_inside_the_footprint_only() {
    let mut device = device();
    let image = device
        .upload_image(&rgba(1, 1, &[[255, 0, 255, 255]]))
        .expect("upload");
    // Rotate 45° and scale by 8√2: the unit square becomes the diamond with
    // vertices (8, 0), (16, 8), (8, 16), (0, 8) after translating by (8, 0).
    let s = 8.0_f32 * std::f32::consts::FRAC_1_SQRT_2 * std::f32::consts::SQRT_2;
    let r = std::f32::consts::FRAC_1_SQRT_2 * s;
    let rotate = Affine {
        a: r,
        b: r,
        c: -r,
        d: r,
        e: 8.0,
        f: 0.0,
    };
    let mut builder = SceneBuilder::new();
    builder
        .image(
            image,
            rotate,
            1.0,
            ImageFilter::Nearest,
            None,
            BlendMode::Normal,
            None,
        )
        .expect("valid image command");
    let pixels = render(&mut device, &builder.finish(), 16, 16);
    assert_eq!(
        pixel(&pixels, 16, 8, 8),
        [255, 0, 255, 255],
        "the diamond's centre is painted"
    );
    assert_eq!(
        pixel(&pixels, 16, 1, 1)[3],
        0,
        "the bbox corner outside the diamond is not"
    );
    assert_eq!(pixel(&pixels, 16, 14, 1)[3], 0, "nor the other corner");
}

/// §8.7.4.5.2's axial sweep on the analytic-rectangle path: t is the projection of
/// the pixel centre onto the axis, the ramp texel is `round(t·255)`, and with a
/// black→white ramp every byte is derivable from the projection alone.
#[test]
fn axial_shading_sweeps_the_projection() {
    let mut device = device();
    let ramp = device.upload_ramp(&grey_ramp()).expect("upload");
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(16.0, 4.0),
        )))
        .expect("upload");
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Shading {
                ramp,
                transform: Affine::IDENTITY,
                kind: ShadingKind::Axial {
                    start: Point::new(0.0, 0.0),
                    end: Point::new(16.0, 0.0),
                    extend: (true, true),
                },
            },
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("valid shaded fill");
    let pixels = render(&mut device, &builder.finish(), 16, 4);
    for x in 0..16 {
        let t = (x as f32 + 0.5) / 16.0;
        let expected = i32::from((t * 255.0).round() as u8);
        let got = pixel(&pixels, 16, x, 2);
        for channel in 0..3 {
            assert!(
                (i32::from(got[channel]) - expected).abs() <= 2,
                "pixel {x}: expected ≈{expected} from §8.7.4.5.2's projection, got {got:?}"
            );
        }
        assert_eq!(got[3], 255);
    }
}

/// §8.7.4.5.2: where extension is off, *nothing* is painted beyond the boundary —
/// not the end colour. With extension on, the boundary colours continue.
#[test]
fn unextended_axial_ends_paint_nothing() {
    let mut device = device();
    let ramp = device.upload_ramp(&grey_ramp()).expect("upload");
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(16.0, 4.0),
        )))
        .expect("upload");
    let shade = |extend: (bool, bool)| {
        let mut builder = SceneBuilder::new();
        builder
            .fill(
                outline,
                Affine::IDENTITY,
                FillRule::NonZero,
                Paint::Shading {
                    ramp,
                    transform: Affine::IDENTITY,
                    kind: ShadingKind::Axial {
                        start: Point::new(4.0, 0.0),
                        end: Point::new(12.0, 0.0),
                        extend,
                    },
                },
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .expect("valid shaded fill");
        builder.finish()
    };

    let unextended = render(&mut device, &shade((false, false)), 16, 4);
    assert_eq!(
        pixel(&unextended, 16, 1, 2)[3],
        0,
        "before an unextended start: no mark at all"
    );
    assert_eq!(
        pixel(&unextended, 16, 14, 2)[3],
        0,
        "past an unextended end: no mark at all"
    );
    assert_eq!(
        pixel(&unextended, 16, 8, 2)[3],
        255,
        "the band itself paints"
    );

    let extended = render(&mut device, &shade((true, true)), 16, 4);
    let before = pixel(&extended, 16, 1, 2);
    assert_eq!(before[3], 255, "an extended start continues");
    assert!(
        before[0] <= 2,
        "with the ramp's first colour, got {before:?}"
    );
    let after = pixel(&extended, 16, 14, 2);
    assert!(after[0] >= 253, "an extended end continues the last colour");
}

/// §8.7.4.5.3's radial sweep: concentric circles from radius 0 to 8 make t the
/// distance over 8, so any probe pixel's byte is derivable from its distance to the
/// centre.
#[test]
fn radial_shading_sweeps_the_distance() {
    let mut device = device();
    let ramp = device.upload_ramp(&grey_ramp()).expect("upload");
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(16.0, 16.0),
        )))
        .expect("upload");
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Shading {
                ramp,
                transform: Affine::IDENTITY,
                kind: ShadingKind::Radial {
                    start: Point::new(8.0, 8.0),
                    start_radius: 0.0,
                    end: Point::new(8.0, 8.0),
                    end_radius: 8.0,
                    extend: (true, true),
                },
            },
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("valid shaded fill");
    let pixels = render(&mut device, &builder.finish(), 16, 16);
    for (x, y) in [(8_u32, 8_u32), (12, 8), (8, 3), (0, 0), (15, 15)] {
        let dx = (x as f32 + 0.5) - 8.0;
        let dy = (y as f32 + 0.5) - 8.0;
        let t = (dx.hypot(dy) / 8.0).min(1.0);
        let expected = i32::from((t * 255.0).round() as u8);
        let got = pixel(&pixels, 16, x, y);
        assert!(
            (i32::from(got[0]) - expected).abs() <= 3,
            "probe ({x}, {y}): distance sweep says ≈{expected}, got {got:?}"
        );
    }
}

/// A shaded fill of a non-rectangular path takes its coverage from the same CPU
/// rasteriser as a solid fill (ADR 0008 feeds ADR 0011): the two alpha planes must
/// agree within one unorm step, and the outside stays empty.
#[test]
fn shading_through_a_triangle_matches_solid_coverage() {
    let mut device = device();
    let ramp = device.upload_ramp(&grey_ramp()).expect("upload");
    let triangle = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.5, 0.5)),
            Segment::LineTo(Point::new(15.5, 0.5)),
            Segment::LineTo(Point::new(0.5, 15.5)),
            Segment::Close,
        ])
        .expect("upload");

    let mut shaded = SceneBuilder::new();
    shaded
        .fill(
            triangle,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Shading {
                ramp,
                transform: Affine::IDENTITY,
                kind: ShadingKind::Axial {
                    start: Point::new(0.0, 0.0),
                    end: Point::new(16.0, 0.0),
                    extend: (true, true),
                },
            },
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("valid shaded fill");
    let shaded_pixels = render(&mut device, &shaded.finish(), 16, 16);

    let mut solid = SceneBuilder::new();
    solid
        .fill(
            triangle,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Solid(Color::new(1.0, 1.0, 1.0, 1.0)),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("valid solid fill");
    let solid_pixels = render(&mut device, &solid.finish(), 16, 16);

    for at in (3..shaded_pixels.len()).step_by(4) {
        assert!(
            (i32::from(shaded_pixels[at]) - i32::from(solid_pixels[at])).abs() <= 1,
            "alpha plane diverged at byte {at}: one rasteriser feeds both lanes"
        );
    }
    assert_eq!(
        pixel(&shaded_pixels, 16, 15, 15)[3],
        0,
        "outside stays empty"
    );
}

/// Integration note 5: a mesh raster is anchored at absolute device pixels — the
/// uploaded left/top place it, the path's coverage gates it, and outside the raster
/// nothing is painted.
#[test]
fn mesh_samples_at_absolute_device_pixels() {
    let mut device = device();
    let mesh = device
        .upload_mesh(&MeshSpec {
            left: 4,
            top: 2,
            image: rgba(4, 4, &[[200, 40, 40, 255]; 16]),
        })
        .expect("upload");
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(16.0, 16.0),
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
        .expect("valid mesh fill");
    let pixels = render(&mut device, &builder.finish(), 16, 16);
    assert_eq!(
        pixel(&pixels, 16, 5, 3),
        [200, 40, 40, 255],
        "inside the anchored raster"
    );
    assert_eq!(
        pixel(&pixels, 16, 2, 2)[3],
        0,
        "left of the raster: nothing"
    );
    assert_eq!(pixel(&pixels, 16, 10, 8)[3], 0, "past the raster: nothing");
}

/// A dangling image, ramp or mesh id is a refusal naming the id — the same contract
/// as an unknown outline, per family (§5: an `Err`, never a hole).
#[test]
fn unknown_paint_ids_are_refused_by_name() {
    let mut device = device();
    let viewport = Viewport::full(4, 4, Affine::IDENTITY);

    let mut with_image = SceneBuilder::new();
    with_image
        .image(
            ImageId(9999),
            Affine::IDENTITY,
            1.0,
            ImageFilter::Nearest,
            None,
            BlendMode::Normal,
            None,
        )
        .expect("the builder cannot know the device's ids");
    assert!(matches!(
        device.render(&with_image.finish(), &viewport, Target::Readback),
        Err(RenderError::UnknownImage { .. })
    ));

    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(4.0, 4.0),
        )))
        .expect("upload");
    let mut with_ramp = SceneBuilder::new();
    with_ramp
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Shading {
                ramp: RampId(9999),
                transform: Affine::IDENTITY,
                kind: ShadingKind::Axial {
                    start: Point::new(0.0, 0.0),
                    end: Point::new(4.0, 0.0),
                    extend: (true, true),
                },
            },
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("valid otherwise");
    assert!(matches!(
        device.render(&with_ramp.finish(), &viewport, Target::Readback),
        Err(RenderError::UnknownRamp { .. })
    ));

    let mut with_mesh = SceneBuilder::new();
    with_mesh
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Mesh(MeshId(9999)),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("valid otherwise");
    assert!(matches!(
        device.render(&with_mesh.finish(), &viewport, Target::Readback),
        Err(RenderError::UnknownMesh { .. })
    ));
}

/// A released image's GPU form goes with its CPU copy: the next frame that
/// references it is refused, not served from a stale texture.
#[test]
fn released_images_do_not_linger_on_the_device() {
    let mut device = device();
    let image = device
        .upload_image(&rgba(1, 1, &[[255, 255, 255, 255]]))
        .expect("upload");
    let mut builder = SceneBuilder::new();
    builder
        .image(
            image,
            Affine::scale(4.0, 4.0),
            1.0,
            ImageFilter::Nearest,
            None,
            BlendMode::Normal,
            None,
        )
        .expect("valid image command");
    let scene = builder.finish();
    let viewport = Viewport::full(4, 4, Affine::IDENTITY);
    device
        .render(&scene, &viewport, Target::Readback)
        .expect("first frame draws");
    device.release(image).expect("resident");
    assert!(matches!(
        device.render(&scene, &viewport, Target::Readback),
        Err(RenderError::UnknownImage { .. })
    ));
}

/// The rare-case lanes ride the clause 11 machinery like every other draw: an
/// opaque image inside a group at α 0.5 composites at half opacity (§11.4.5), which
/// is derivable without reference to any other renderer.
#[test]
fn image_inside_a_group_takes_the_group_alpha() {
    let mut device = device();
    let image = device
        .upload_image(&rgba(1, 1, &[[255, 255, 255, 255]]))
        .expect("upload");
    let mut builder = SceneBuilder::new();
    builder
        .group(
            quorra_scene::GroupSpec {
                alpha: 0.5,
                blend: BlendMode::Normal,
                clip: None,
                knockout: false,
                mask: None,
                isolated: true,
                compose: Compose::SrcOver,
            },
            |body| {
                body.image(
                    image,
                    Affine::scale(4.0, 4.0),
                    1.0,
                    ImageFilter::Nearest,
                    None,
                    BlendMode::Normal,
                    None,
                )
            },
        )
        .expect("valid group");
    let pixels = render(&mut device, &builder.finish(), 4, 4);
    let got = pixel(&pixels, 4, 2, 2);
    assert!(
        (i32::from(got[3]) - 128).abs() <= 2,
        "group alpha applies once at the composite, got {got:?}"
    );
}

/// ADR 0006's cross-adapter bound extends to the deterministic M7 paths: nearest
/// images and CPU-sampled ramps are wholly our arithmetic, so llvmpipe and RADV may
/// differ only by the float→unorm store rounding (≤ 2 unorm steps). Linear
/// filtering is deliberately absent here — its variance is the sampler's, stated in
/// ADR 0011 and gated by shape in its own test.
#[test]
fn cross_adapter_bound_holds_for_the_deterministic_paths() {
    let adapters: Vec<String> = ["llvmpipe", "RADV"]
        .iter()
        .filter_map(|name| {
            Device::headless(&Options {
                adapter: Some((*name).into()),
                ..Options::default()
            })
            .ok()
            .map(|_| (*name).to_string())
        })
        .collect();
    if adapters.len() < 2 {
        eprintln!("note: only one adapter; the cross-adapter check compared nothing");
        return;
    }

    let rasters: Vec<Vec<u8>> = adapters
        .iter()
        .map(|name| {
            let mut device = Device::headless(&Options {
                adapter: Some(name.clone()),
                ..Options::default()
            })
            .expect("probed above");
            let image = device
                .upload_image(&rgba(
                    2,
                    2,
                    &[
                        [255, 0, 0, 255],
                        [0, 255, 0, 200],
                        [0, 0, 255, 150],
                        [255, 255, 0, 100],
                    ],
                ))
                .expect("upload");
            let ramp = device.upload_ramp(&grey_ramp()).expect("upload");
            let outline = device
                .upload_outline(&rect_outline(Rect::new(
                    Point::new(1.0, 9.0),
                    Point::new(15.0, 15.0),
                )))
                .expect("upload");
            let mut builder = SceneBuilder::new();
            builder
                .image(
                    image,
                    Affine::scale(16.0, 8.0),
                    0.8,
                    ImageFilter::Nearest,
                    None,
                    BlendMode::Normal,
                    None,
                )
                .expect("valid image command");
            builder
                .fill(
                    outline,
                    Affine::IDENTITY,
                    FillRule::NonZero,
                    Paint::Shading {
                        ramp,
                        transform: Affine::IDENTITY,
                        kind: ShadingKind::Radial {
                            start: Point::new(8.0, 12.0),
                            start_radius: 0.0,
                            end: Point::new(8.0, 12.0),
                            end_radius: 6.0,
                            extend: (true, true),
                        },
                    },
                    None,
                    BlendMode::Normal,
                    Compose::SrcOver,
                    None,
                )
                .expect("valid shaded fill");
            render(&mut device, &builder.finish(), 16, 16)
        })
        .collect();

    let diff = rasters[0]
        .iter()
        .zip(&rasters[1])
        .map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs())
        .max()
        .unwrap_or(0);
    assert!(
        diff <= 2,
        "deterministic M7 paths diverged across adapters by {diff} unorm steps (ADR 0006 bound: 2)"
    );
}

/// Determinism on one adapter (§4.6): the same M7 scene renders byte-identically
/// twice — nearest images and CPU-sampled ramps leave the driver nothing to vary.
#[test]
fn m7_frames_are_deterministic_per_adapter() {
    let mut device = device();
    let image = device
        .upload_image(&rgba(
            2,
            2,
            &[
                [255, 0, 0, 255],
                [0, 255, 0, 200],
                [0, 0, 255, 150],
                [255, 255, 0, 100],
            ],
        ))
        .expect("upload");
    let ramp = device.upload_ramp(&grey_ramp()).expect("upload");
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(1.0, 1.0),
            Point::new(15.0, 9.0),
        )))
        .expect("upload");
    let mut builder = SceneBuilder::new();
    builder
        .image(
            image,
            Affine::scale(16.0, 16.0),
            0.8,
            ImageFilter::Nearest,
            None,
            BlendMode::Normal,
            None,
        )
        .expect("valid image command");
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Shading {
                ramp,
                transform: Affine::IDENTITY,
                kind: ShadingKind::Axial {
                    start: Point::new(0.0, 0.0),
                    end: Point::new(16.0, 4.0),
                    extend: (true, false),
                },
            },
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("valid shaded fill");
    let scene = builder.finish();
    let first = render(&mut device, &scene, 16, 16);
    let second = render(&mut device, &scene, 16, 16);
    assert_eq!(first, second, "same adapter, same scene, same bytes");
}
