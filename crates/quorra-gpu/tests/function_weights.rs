//! A §7.10.5 program painted under a **clip** and under a **soft mask** (ADR 0053).
//!
//! # Why this file exists
//!
//! `function_lane.wgsl`'s fragment stage weights the paint by `base_weight`, which is
//! `coverage × clip × soft mask` — three factors, and until this file **only the first was
//! ever anything but 1** anywhere in the tree (`doc/notes-function-wiring.md` §4.5). The
//! line is textually the shading lane's, which is an argument that it works and is not
//! evidence that it does: a lane that dropped the clip factor, or sampled the mask through
//! the wrong placement, draws a plausible page and every existing function test stays green.
//!
//! # Where the expected values come from
//!
//! Each test states its own clause; the three the file rests on are:
//!
//! - **ISO 32000-2 §8.5.4** for the clip. A chain is one region arrived at by intersection
//!   — the clause's own sentence is quoted in `encode/clips.rs` — so a mark is painted on
//!   `shape ∩ clip` and nowhere else. Where a clip's edge falls *inside* a pixel, the
//!   fraction the clause admits is the area of that pixel inside the region, which is the
//!   same quantity `coverage` already means everywhere in this tree (§4.1 of the brief:
//!   a renderer that answered 0 or 1 there would agree with the clause only on
//!   whole-pixel boundaries).
//! - **§11.5.2** for an alpha mask: the mask value is derived from the group's alpha.
//! - **§11.5.3** for a luminosity mask: the group is composited with a fully opaque
//!   backdrop of a specified colour and the mask value is the luminosity of the result.
//!   Outside the part of the page the mask's group marks, the value is what the reduction
//!   writes for a fully transparent pixel (ADR 0037) — 0 under §11.5.2, and the backdrop's
//!   own luminosity under §11.5.3.
//!
//! Every colour expectation is the function's value at the pixel's **centre** (§10.7.4)
//! weighted by the factor under test, compared in the premultiplied space the weight is
//! applied in and to the byte ADR 0006's store leaves.

// Test-file lint policy as in m1.rs; the reference math mirrors clause arithmetic.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    // Pixel indexing and the clause's own arithmetic, over rasters this file just drew.
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, ClipId, Color, Compose, FillRule, FnOp, FnRange, FunctionId, MaskId,
    MaskKind, Paint, Point, Rect, Scene, SceneBuilder, Segment,
};

/// 64 pixels wide: 64 × 4 bytes = 256, the buffer-copy row alignment.
const SIZE: u32 = 64;

/// `x y 0.5` — the shortest program that is a function of both inputs and leaves three
/// values, so the colour at a point *is* its position. A paint of one constant colour
/// would let a weight be checked while a displaced quad went unnoticed.
const POSITION_IS_COLOUR: [FnOp; 1] = [FnOp::PushReal(0.5)];

/// Where the rectangular clip's right edge falls: **inside** a pixel, so the column at
/// x = 40 is admitted by a quarter and the clip is observed as a weight rather than as a
/// cut at a pixel boundary. Chosen at a quarter because 0.25 × 255 = 63.75 rounds to a
/// byte no other factor in this file produces.
const CLIP_EDGE: f32 = 40.25;

/// The device this file's assertions are made on, named so a failure says where: ADR 0053
/// promises **no** cross-adapter identity for this paint, so every message carries the
/// adapter. `QUORRA_ADAPTER` picks another; the suite's default is the software
/// rasteriser, as everywhere else in this tree.
fn device() -> (Device, String) {
    let requested = std::env::var("QUORRA_ADAPTER").unwrap_or_else(|_| "llvmpipe".into());
    let device = Device::headless(&Options {
        adapter: Some(requested),
        ..Options::default()
    })
    .expect("the requested adapter is present");
    let name = device.description().to_string();
    (device, name)
}

fn render(device: &mut Device, scene: &Scene) -> Vec<u8> {
    device
        .render(
            scene,
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the frame is drawn")
        .into_raster()
        .unwrap()
        .into_pixels()
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * SIZE + x) * 4) as usize;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

/// One channel of a readback pixel back in the premultiplied space the weight was applied
/// in, as a byte of 255.
///
/// The weight multiplies the *premultiplied* colour and the alpha together, so it is
/// invisible in a straight-alpha channel and visible in this one — which is why every
/// colour assertion below is made here rather than on the raw byte.
fn premul(pixels: &[u8], x: u32, y: u32, channel: usize) -> f32 {
    let got = pixel(pixels, x, y);
    f32::from(got[channel]) * f32::from(got[3]) / 255.0
}

/// A closed rectangle as four axis-aligned edges — what `rect_hint` recognises, and so
/// what makes a fill take the analytic lane and a clip link collapse into the resolved
/// clip rectangle rather than into a residue.
fn rect_outline(rect: Rect) -> Vec<Segment> {
    vec![
        Segment::MoveTo(rect.min),
        Segment::LineTo(Point::new(rect.max.x, rect.min.y)),
        Segment::LineTo(rect.max),
        Segment::LineTo(Point::new(rect.min.x, rect.max.y)),
        Segment::Close,
    ]
}

/// The whole target as a closed rectangle: the fill every test here paints.
fn full_rect(device: &mut Device) -> quorra_scene::OutlineId {
    device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(SIZE as f32, SIZE as f32),
        )))
        .expect("upload")
}

/// The unit square: §8.7.4.5.2's `Domain`, mapped onto the whole target by the `Matrix`
/// below, so the paint marks every pixel and the only thing that can weight it is the
/// factor under test.
fn unit_square() -> Rect {
    Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0))
}

/// One function-painted fill of `outline`, under an optional clip and an optional soft
/// mask. The `Matrix` maps the unit square onto the target, so the value at a pixel is its
/// own position (§10.7.4 for the centre).
fn function_fill(
    builder: &mut SceneBuilder,
    outline: quorra_scene::OutlineId,
    program: FunctionId,
    clip: Option<ClipId>,
    mask: Option<MaskId>,
) {
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Function {
                program,
                domain: unit_square(),
                matrix: Affine::scale(SIZE as f32, SIZE as f32),
                range: FnRange::Rgb([[0.0, 1.0]; 3]),
                background: None,
            },
            clip,
            BlendMode::Normal,
            Compose::SrcOver,
            mask,
        )
        .expect("a valid function fill");
}

/// The function's own value at the centre of pixel `(x, y)`, per channel — `x y 0.5` under
/// a `Matrix` that scales the unit square onto the target.
fn value_at(x: u32, y: u32, channel: usize) -> f32 {
    match channel {
        0 => (x as f32 + 0.5) / SIZE as f32,
        1 => (y as f32 + 0.5) / SIZE as f32,
        _ => 0.5,
    }
}

/// Assert that the paint at `(x, y)` is the function's value weighted by `weight`.
///
/// Both halves of `base_weight`'s effect are checked: the alpha *is* the weight (the paint
/// is opaque inside its domain), and each premultiplied channel is the value times it. A
/// lane that weighted only the alpha, or only the colour, passes neither.
fn assert_weighted(pixels: &[u8], x: u32, y: u32, weight: f32, what: &str) {
    let alpha = (weight * 255.0).round() as i32;
    let got = pixel(pixels, x, y);
    assert!(
        (i32::from(got[3]) - alpha).abs() <= 1,
        "{what}: the weight is the paint's alpha — expected {alpha} ± 1, got {got:?}"
    );
    for channel in 0..3 {
        let expected = value_at(x, y, channel) * weight * 255.0;
        let got = premul(pixels, x, y, channel);
        assert!(
            (got - expected).abs() <= 1.5,
            "{what}: premultiplied channel {channel} is the function's value at the \
             pixel's centre times the weight — expected {expected:.2} ± 1.5 of 255, got \
             {got:.2}"
        );
    }
}

/// **A rectangular clip is a factor of the weight, not a cut at a pixel boundary.**
///
/// ISO 32000-2 §8.5.4 sets the clipping path to the intersection of the current path and
/// the new one, and §8.5.4's whole subject is that painting affects that region and no
/// other. The clip here is `x < 40.25`, so:
///
/// | column | admitted | why |
/// |---|---|---|
/// | 20 | all of it | wholly inside the region |
/// | 40 | a quarter | the region's edge crosses the pixel at 40.25 |
/// | 41 | none | wholly outside |
///
/// The fill is the whole target and the outline is four axis-aligned edges, so this draws
/// the **rect-hinted** placement, where the quad is the shape's device rectangle cut to the
/// clip and rounded out — and the fractional quarter therefore has to come from the
/// shader's weight, because the geometry cannot express it.
#[test]
fn a_rectangular_clip_weights_the_paint_by_the_area_it_admits() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("`x y 0.5` is admitted");
    let outline = full_rect(&mut device);
    let clip_shape = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(CLIP_EDGE, SIZE as f32),
        )))
        .expect("upload");

    let mut builder = SceneBuilder::new();
    let clip = builder
        .clip(clip_shape, Affine::IDENTITY, FillRule::NonZero, None)
        .expect("a valid clip");
    function_fill(&mut builder, outline, program, Some(clip), None);
    let pixels = render(&mut device, &builder.finish());

    for y in [8_u32, 32, 55] {
        assert_weighted(
            &pixels,
            20,
            y,
            1.0,
            &format!("{adapter}: (20, {y}) is wholly inside the clip"),
        );
        assert_weighted(
            &pixels,
            40,
            y,
            0.25,
            &format!("{adapter}: (40, {y}) is a quarter inside the clip's region"),
        );
        assert_eq!(
            pixel(&pixels, 41, y),
            [0, 0, 0, 0],
            "{adapter}: (41, {y}) is outside the region §8.5.4 intersects to, so the \
             clause paints nothing there"
        );
    }
}

/// **A non-rectangular clip reaches this paint too**, and it reaches it through the
/// coverage tile rather than through the clip rectangle.
///
/// A clip link that is not a rectangle under its own transform cannot be intersected into
/// the resolved clip rectangle, so §8.5.4's intersection is taken by rasterising the link
/// and multiplying it into the mark's coverage — which also means a **rect-hinted** fill
/// under such a clip stops being rect-hinted and takes the rasterised-coverage placement.
/// `Counters::tiles` says so, and is asserted, because that reroute is what this test is
/// about as much as the pixels are.
///
/// The expectation is the clause's intersection read off the device *independently*: the
/// same outline under the same clip, painted opaque white, has alpha equal to the region
/// §8.5.4 admits at each pixel — geometry alone, no function involved — and the function
/// fill must carry exactly that as its weight. Where the diamond admits everything the
/// colour is the function's own value, and where it admits nothing the clause paints
/// nothing.
#[test]
fn a_residue_clip_weights_the_paint_by_the_region_the_clause_intersects() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let outline = full_rect(&mut device);
    // A diamond: not a rectangle under any axis-preserving transform, so the chain keeps
    // it as a residue.
    let diamond = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(32.0, 4.0)),
            Segment::LineTo(Point::new(60.0, 32.0)),
            Segment::LineTo(Point::new(32.0, 60.0)),
            Segment::LineTo(Point::new(4.0, 32.0)),
            Segment::Close,
        ])
        .expect("upload");

    let scene_of = |paint_with_program: bool| {
        let mut builder = SceneBuilder::new();
        let clip = builder
            .clip(diamond, Affine::IDENTITY, FillRule::NonZero, None)
            .expect("a valid clip");
        if paint_with_program {
            function_fill(&mut builder, outline, program, Some(clip), None);
        } else {
            builder
                .fill(
                    outline,
                    Affine::IDENTITY,
                    FillRule::NonZero,
                    Paint::Solid(Color::new(1.0, 1.0, 1.0, 1.0)),
                    Some(clip),
                    BlendMode::Normal,
                    Compose::SrcOver,
                    None,
                )
                .expect("a valid solid fill");
        }
        builder.finish()
    };

    let painted = device
        .render(
            &scene_of(true),
            &Viewport::full(SIZE, SIZE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the frame is drawn");
    let tiles = painted.counters().tiles;
    let painted = painted.into_raster().unwrap().into_pixels();
    assert_eq!(
        tiles, 1,
        "{adapter}: a residue clip has nowhere to go in the analytic lane, so even a \
         rect-hinted function fill becomes one rasterised coverage tile"
    );

    // The region §8.5.4 admits, measured by geometry alone.
    let region = render(&mut device, &scene_of(false));

    let mut worst = 0_u32;
    let mut partial = 0_u32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let admitted = pixel(&region, x, y)[3];
            if admitted > 0 && admitted < 255 {
                partial += 1;
            }
            worst = worst.max(u32::from(pixel(&painted, x, y)[3].abs_diff(admitted)));
        }
    }
    assert!(
        partial > 100,
        "{adapter}: the diamond's edges must cross pixels for this to be about a weight \
         rather than about a mask of 0 and 1: {partial}"
    );
    assert!(
        worst <= 1,
        "{adapter}: the function paint is opaque inside its domain, so its alpha is the \
         region the clip admits; worst difference {worst} of 255"
    );

    // And the colour inside is still the function's own, so the residue weights the paint
    // rather than replacing it.
    assert_weighted(
        &painted,
        32,
        32,
        1.0,
        &format!("{adapter}: the middle of the diamond"),
    );
    assert_eq!(
        pixel(&painted, 4, 4),
        [0, 0, 0, 0],
        "{adapter}: a corner the diamond clips away"
    );
}

/// A soft mask whose group marks the left half of the target at `alpha`, under §11.5.2's
/// rule — so the mask value is `alpha` there and the reduction of a transparent pixel
/// (which §11.5.2 derives as 0) everywhere else.
fn alpha_mask(builder: &mut SceneBuilder, alpha: f32) -> MaskId {
    builder
        .mask(MaskKind::Alpha, None, |body| {
            body.rect(
                Rect::new(
                    Point::new(0.0, 0.0),
                    Point::new(SIZE as f32 / 2.0, SIZE as f32),
                ),
                Affine::IDENTITY,
                Color::new(1.0, 1.0, 1.0, alpha),
                None,
                None,
            )
        })
        .expect("a valid mask")
}

/// **§11.5.2's mask value weights this paint** — and outside the mask's own group it is
/// the reduction of a transparent pixel, which admits nothing.
///
/// > The mask value at each point shall then be derived from the alpha of the group
///
/// (§11.5.2, as `reduce.wgsl` quotes it.) The group's only mark is opaque white at alpha
/// 0.4 over the left half, so the mask is 0.4 there — 0.4 × 255 = 102 exactly, which is
/// why that alpha and no other — and §11.5.2 derives **0** from the transparent right
/// half (ADR 0037's constant, held by `tests/mask_regions.rs` for every rule and
/// transfer). So the function paint is weighted by 0.4 on the left and painted nowhere on
/// the right, while its colour is unchanged in both.
///
/// The mask's group covers half the target and the fill covers all of it, so this also
/// holds the *placement*: a lane that sampled the mask at the quad's own origin rather
/// than at the device pixel would put the boundary somewhere else.
#[test]
fn an_alpha_soft_mask_weights_the_paint_by_11_5_2s_mask_value() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let outline = full_rect(&mut device);

    let mut builder = SceneBuilder::new();
    let mask = alpha_mask(&mut builder, 0.4);
    function_fill(&mut builder, outline, program, None, Some(mask));
    let pixels = render(&mut device, &builder.finish());

    for y in [8_u32, 32, 55] {
        assert_weighted(
            &pixels,
            10,
            y,
            0.4,
            &format!("{adapter}: (10, {y}), where §11.5.2 derives 0.4 from the group"),
        );
        assert_eq!(
            pixel(&pixels, 50, y),
            [0, 0, 0, 0],
            "{adapter}: at (50, {y}) the mask's group marks nothing, so §11.5.2 derives \
             0 and the mask admits nothing"
        );
    }
    // The boundary is the group's, to the pixel.
    assert_eq!(
        pixel(&pixels, 31, 32)[3],
        pixel(&pixels, 10, 32)[3],
        "{adapter}: the last column the mask's group marks"
    );
    assert_eq!(
        pixel(&pixels, 32, 32),
        [0, 0, 0, 0],
        "{adapter}: and the first column it does not"
    );
}

/// **§11.5.3's luminosity weights this paint**, coefficients and backdrop included.
///
/// > the group shall be composited with a fully opaque backdrop of the colour specified
/// > by the **BC** entry […] the mask value at each point shall be the luminosity of the
/// > result
///
/// (§11.5.3, as `reduce.wgsl` implements it and `quorra-scene`'s `MaskKind` documents it.)
/// The group's mark is opaque `(0.2, 0.4, 0.6)` — 51, 102 and 153, all exact in 8 bits —
/// so inside it the composite is that colour and the mask is
/// `(0.30 × 51 + 0.59 × 102 + 0.11 × 153) / 255 = 92.31 / 255`. Outside it the group is
/// transparent, the composite is the backdrop alone, and the backdrop here is **white**:
/// the mask admits everything, which is the case a lane that assumed zero outside a mask's
/// rectangle would get exactly backwards.
///
/// So this one fixture carries both directions of ADR 0037's constant *and* the clause's
/// coefficients: a grey mark would have given the same answer under any three weights
/// summing to one.
#[test]
fn a_luminosity_soft_mask_weights_the_paint_by_11_5_3s_luminosity() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let outline = full_rect(&mut device);
    let group_colour = Color::new(0.2, 0.4, 0.6, 1.0);

    let mut builder = SceneBuilder::new();
    let mask = builder
        .mask(
            MaskKind::Luminosity {
                backdrop: Color::new(1.0, 1.0, 1.0, 1.0),
            },
            None,
            |body| {
                body.rect(
                    Rect::new(
                        Point::new(0.0, 0.0),
                        Point::new(SIZE as f32 / 2.0, SIZE as f32),
                    ),
                    Affine::IDENTITY,
                    group_colour,
                    None,
                    None,
                )
            },
        )
        .expect("a valid mask");
    function_fill(&mut builder, outline, program, None, Some(mask));
    let pixels = render(&mut device, &builder.finish());

    // §11.5.3's coefficients, over the bytes the group's colour is stored as.
    let inside = 0.30_f32.mul_add(
        f32::from((group_colour.r * 255.0) as u8),
        0.59_f32.mul_add(
            f32::from((group_colour.g * 255.0) as u8),
            0.11 * f32::from((group_colour.b * 255.0) as u8),
        ),
    ) / 255.0;
    for y in [8_u32, 32, 55] {
        assert_weighted(
            &pixels,
            10,
            y,
            inside,
            &format!("{adapter}: (10, {y}), inside the mask's group"),
        );
        assert_weighted(
            &pixels,
            50,
            y,
            1.0,
            &format!(
                "{adapter}: (50, {y}), outside it — the group is transparent there, so \
                 §11.5.3 reduces the opaque white backdrop alone, whose luminosity is 1"
            ),
        );
    }
}

/// **The clip and the mask are two factors of one product**, which is the statement
/// `base_weight` makes and the one a lane using either in place of the other fails.
///
/// The clip admits a quarter of column 40 (§8.5.4) and the mask admits 0.4 everywhere its
/// group marks (§11.5.2), so the weight there is `0.25 × 0.4 = 0.1` — a byte no single
/// factor of this fixture produces, and the reason the two are drawn together rather than
/// only apart.
#[test]
fn a_clip_and_a_soft_mask_multiply() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let outline = full_rect(&mut device);
    let clip_shape = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(CLIP_EDGE, SIZE as f32),
        )))
        .expect("upload");

    let mut builder = SceneBuilder::new();
    let mask = builder
        .mask(MaskKind::Alpha, None, |body| {
            body.rect(
                Rect::new(Point::new(0.0, 0.0), Point::new(SIZE as f32, SIZE as f32)),
                Affine::IDENTITY,
                Color::new(1.0, 1.0, 1.0, 0.4),
                None,
                None,
            )
        })
        .expect("a valid mask");
    let clip = builder
        .clip(clip_shape, Affine::IDENTITY, FillRule::NonZero, None)
        .expect("a valid clip");
    function_fill(&mut builder, outline, program, Some(clip), Some(mask));
    let pixels = render(&mut device, &builder.finish());

    assert_weighted(
        &pixels,
        20,
        32,
        0.4,
        &format!("{adapter}: (20, 32), inside the clip and under the mask"),
    );
    assert_weighted(
        &pixels,
        40,
        32,
        0.25 * 0.4,
        &format!("{adapter}: (40, 32), a quarter of the clip's region under the mask"),
    );
    assert_eq!(
        pixel(&pixels, 41, 32),
        [0, 0, 0, 0],
        "{adapter}: and outside the clip the mask has nothing to weight"
    );
}
