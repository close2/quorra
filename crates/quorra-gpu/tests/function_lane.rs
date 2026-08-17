//! A §7.10.5 program painted through a real frame (ADR 0053), held against the clause.
//!
//! `function_device.rs` runs the generated arithmetic in a compute pass, where no pixel
//! format sits between the shader and the assertion. **This file is the other half**: the
//! program inside a `Device::render`, through the encoder's placement, the clip, the
//! coverage tile and ADR 0006's 8-bit store — the path a page actually takes, and the one
//! where a lane that is wired wrongly draws something plausible.
//!
//! # What each expectation is derived from
//!
//! The colours are arithmetic: the program is `x y 0.5`, the `Matrix` scales the unit
//! square onto the shape, so the pixel at device `(i, j)` carries the function's value at
//! its own centre (ISO 32000-2 §10.7.4) and the expected byte is that value stored per
//! ADR 0006. Nothing here is a number copied out of a previous run.
//!
//! # The two shapes the caller's corpus cannot show
//!
//! Both of the viewer's witnesses declare `/Domain [0 1 0 1]` and `/Range [0 1 0 1 0 1]`,
//! so no corpus run can distinguish the clause from three plausible misreadings of it. Two
//! tests here are those shapes and exist for that reason:
//! `a_domain_smaller_than_the_shape_leaves_the_outside_alone` and
//! `a_range_that_is_not_the_unit_interval_clips_to_its_own_bounds`.

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

use quorra_gpu::{Device, Options, RenderError, ReportKind, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, FnOp, FnRange, FunctionId, Paint, Point, Rect,
    Scene, SceneBuilder, Segment,
};

mod common;

use common::probe::pixel;

/// The device this file's assertions are made on, named so a failure says where.
///
/// `QUORRA_ADAPTER` picks another: the suite runs on the software rasteriser by default,
/// as every other test file here does, and ADR 0053's consequence — cross-adapter identity
/// is **not** promised for this paint — is why the name is in every message.
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

fn rect_outline(rect: Rect) -> Vec<Segment> {
    vec![
        Segment::MoveTo(rect.min),
        Segment::LineTo(Point::new(rect.max.x, rect.min.y)),
        Segment::LineTo(rect.max),
        Segment::LineTo(Point::new(rect.min.x, rect.max.y)),
        Segment::Close,
    ]
}

/// A closed triangle, so that a fill takes the rasterised-coverage path rather than the
/// rect-hinted one: the two place the quad differently and only one of them is exercised
/// by a rectangle.
fn triangle(side: f32) -> Vec<Segment> {
    vec![
        Segment::MoveTo(Point::new(0.0, 0.0)),
        Segment::LineTo(Point::new(side, 0.0)),
        Segment::LineTo(Point::new(0.0, side)),
        Segment::Close,
    ]
}

fn render(device: &mut Device, scene: &Scene, width: u32, height: u32) -> Vec<u8> {
    device
        .render(
            scene,
            &Viewport::full(width, height, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the frame is drawn")
        .into_raster()
        .unwrap()
        .into_pixels()
}

/// `x y 0.5` — the shortest program that is a function of both inputs and leaves three
/// values, so the colour at a point *is* its position and a lane that placed the quad
/// wrongly cannot produce it by accident.
const POSITION_IS_COLOUR: [FnOp; 1] = [FnOp::PushReal(0.5)];

/// A fill of `outline` under a function paint whose domain is the unit square mapped onto
/// a `side`-pixel square at the origin.
fn scene_of(
    program: FunctionId,
    outline: quorra_scene::OutlineId,
    side: f32,
    domain: Rect,
    range: FnRange,
    background: Option<Color>,
) -> Scene {
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Function {
                program,
                domain,
                // §8.7.4.5.2's Matrix: the domain rectangle into the target space. The
                // unit square becomes the shape, so shading space is the shape's own
                // fraction and the expected colours are arithmetic.
                matrix: Affine::scale(side, side),
                range,
                background,
            },
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("a valid function fill");
    builder.finish()
}

/// The byte ADR 0006's store leaves for a straight-alpha component, and the tolerance the
/// spike measured for that step alone (246 044 texels off by one on one adapter).
fn assert_component(got: u8, want: f32, what: &str) {
    let expected = (want.clamp(0.0, 1.0) * 255.0).round() as i32;
    assert!(
        (i32::from(got) - expected).abs() <= 1,
        "{what}: expected {expected} ± 1 from the clause, got {got}"
    );
}

/// **The lane draws.** A program of both inputs, through a rect-hinted fill: every pixel
/// carries the function's value at its own centre (§10.7.4), which is its own position.
#[test]
fn a_function_paint_colours_every_pixel_by_its_own_position() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("`x y 0.5` is admitted");
    let side = 16.0;
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(side, side),
        )))
        .expect("upload");
    let scene = scene_of(
        program,
        outline,
        side,
        Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
        FnRange::Rgb([[0.0, 1.0]; 3]),
        None,
    );
    let pixels = render(&mut device, &scene, 16, 16);

    for y in 0..16 {
        for x in 0..16 {
            let got = pixel(&pixels, 16, x, y);
            let at = format!("{adapter}: pixel ({x}, {y})");
            assert_component(got[0], (x as f32 + 0.5) / side, &format!("{at} red"));
            assert_component(got[1], (y as f32 + 0.5) / side, &format!("{at} green"));
            assert_component(got[2], 0.5, &format!("{at} blue"));
            assert_eq!(got[3], 255, "{at}: inside the domain is fully covered");
        }
    }
}

/// The same program through a **rasterised coverage tile** rather than the rect-hinted
/// fast path: a triangle's interior carries the same colours, and its outside is untouched.
///
/// The two paths compute the quad's destination differently — one from the shape's device
/// rectangle, one from the tile — and the shader's texel arithmetic depends on which. A
/// lane wired for only one of them draws the other displaced.
#[test]
fn the_coverage_path_paints_the_same_colours_as_the_rect_path() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let side = 16.0;
    let outline = device.upload_outline(&triangle(side)).expect("upload");
    let scene = scene_of(
        program,
        outline,
        side,
        Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
        FnRange::Rgb([[0.0, 1.0]; 3]),
        None,
    );
    let pixels = render(&mut device, &scene, 16, 16);

    // Well inside the triangle (x + y < side), and well outside it.
    let inside = pixel(&pixels, 16, 2, 2);
    assert_component(inside[0], 2.5 / side, &format!("{adapter} inside red"));
    assert_component(inside[1], 2.5 / side, &format!("{adapter} inside green"));
    assert_eq!(inside[3], 255);
    assert_eq!(
        pixel(&pixels, 16, 14, 14),
        [0, 0, 0, 0],
        "{adapter}: outside the shape the quad must mark nothing"
    );
}

/// **ISO 32000-2 §8.7.4.5.2's domain rule, in a frame.**
///
/// > Points within the shading's bounding box (BBox) that fall outside this transformed
/// > domain rectangle shall be painted with the shading's background colour (Background);
/// > if the shading dictionary has no Background entry, such points shall be left
/// > unpainted.
///
/// The domain is the left quarter of the shape, so three quarters of the fill is outside
/// it. With no `Background` those pixels must be **untouched**; with one they must be
/// **exactly** it; and in neither case may they be the colour of the nearest domain edge,
/// which is what a shader that clamped its position would have painted.
///
/// No corpus run can see this: both of the caller's witnesses declare `/Domain [0 1 0 1]`
/// mapped onto the shape they paint, so clamping and discarding agree at every pixel.
#[test]
fn a_domain_smaller_than_the_shape_leaves_the_outside_alone() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let side = 16.0;
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(side, side),
        )))
        .expect("upload");
    let quarter = Rect::new(Point::new(0.0, 0.0), Point::new(0.25, 1.0));
    // What a clamping shader would paint at x = 12: the domain's right edge, x = 0.25.
    let clamped_red = (0.25_f32 * 255.0).round() as u8;

    let unpainted = scene_of(
        program,
        outline,
        side,
        quarter,
        FnRange::Rgb([[0.0, 1.0]; 3]),
        None,
    );
    let pixels = render(&mut device, &unpainted, 16, 16);
    assert_component(
        pixel(&pixels, 16, 2, 8)[0],
        2.5 / side,
        &format!("{adapter}: inside the domain"),
    );
    let outside = pixel(&pixels, 16, 12, 8);
    assert_eq!(
        outside,
        [0, 0, 0, 0],
        "{adapter}: with no Background the clause leaves the point unpainted"
    );
    assert_ne!(
        outside[0], clamped_red,
        "{adapter}: a clamped position would have smeared the domain's edge colour here"
    );

    // 0.2, 0.4 and 0.6 are exact in 8 bits (51, 102, 153), so the background can be
    // asserted to the byte rather than to a tolerance.
    let background = Color::new(0.2, 0.4, 0.6, 1.0);
    let painted = scene_of(
        program,
        outline,
        side,
        quarter,
        FnRange::Rgb([[0.0, 1.0]; 3]),
        Some(background),
    );
    let pixels = render(&mut device, &painted, 16, 16);
    assert_eq!(
        pixel(&pixels, 16, 12, 8),
        [51, 102, 153, 255],
        "{adapter}: with a Background the clause paints exactly it"
    );
    assert_component(
        pixel(&pixels, 16, 2, 8)[0],
        2.5 / side,
        &format!("{adapter}: inside the domain, unchanged by the background"),
    );
}

/// **ISO 32000-2 §7.10.1's output clip, against a `Range` that is not the unit interval.**
///
/// > output values produced by the function shall be clipped to the range
///
/// The program leaves `(x, y, 0.5)`; the range clips red into `[0.25, 0.75]`, so the left
/// quarter of the shape is flat at 0.25 and the right quarter flat at 0.75 while the middle
/// is unchanged. Green and blue keep the unit interval, which is what makes this a test of
/// the clip rather than of the shader's constant.
///
/// Invisible to the corpus for the same reason as the domain rule: both witnesses declare
/// `/Range [0 1 0 1 0 1]`, where the clip is a no-op at every pixel.
#[test]
fn a_range_that_is_not_the_unit_interval_clips_to_its_own_bounds() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let side = 16.0;
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(side, side),
        )))
        .expect("upload");
    let scene = scene_of(
        program,
        outline,
        side,
        Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
        FnRange::Rgb([[0.25, 0.75], [0.0, 1.0], [0.0, 1.0]]),
        None,
    );
    let pixels = render(&mut device, &scene, 16, 16);

    for (x, want) in [(0_u32, 0.25_f32), (1, 0.25), (8, 8.5 / side), (15, 0.75)] {
        assert_component(
            pixel(&pixels, 16, x, 8)[0],
            want,
            &format!("{adapter}: red at x = {x}, clipped into [0.25, 0.75]"),
        );
    }
    // The other two components keep their own bounds, so the clip is per component and
    // not one rule applied to the colour.
    assert_component(
        pixel(&pixels, 16, 0, 15)[1],
        15.5 / side,
        &format!("{adapter}: green is clipped by its own pair"),
    );
}

/// One program, two placements, one upload: the domain, matrix, range and background live
/// on the paint, so two shadings share a program and the shader compiled for it.
#[test]
fn two_placements_of_one_program_draw_their_own_geometry() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(8.0, 8.0),
        )))
        .expect("upload");
    let mut builder = SceneBuilder::new();
    for (offset, side) in [(0.0_f32, 8.0_f32), (8.0, 8.0)] {
        builder
            .fill(
                outline,
                Affine::translate(offset, 0.0),
                FillRule::NonZero,
                Paint::Function {
                    program,
                    domain: Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
                    matrix: Affine::scale(side, side).then(Affine::translate(offset, 0.0)),
                    range: FnRange::Rgb([[0.0, 1.0]; 3]),
                    background: None,
                },
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .expect("a valid function fill");
    }
    let pixels = render(&mut device, &builder.finish(), 16, 8);

    // Each half sweeps its own unit square, so the same fraction appears twice.
    for x in [0_u32, 8] {
        assert_component(
            pixel(&pixels, 16, x, 4)[0],
            0.5 / 8.0,
            &format!("{adapter}: the left edge of the placement at x = {x}"),
        );
    }
}

/// A program that reads an empty operand stack draws, and the frame **says so** —
/// ADR 0053 requires that adopting a value ISO 32000-2 does not define is never silent.
///
/// The wording is the count the analyser can supply: how many such reads the program
/// *contains*, not how many the frame's pixels took. See `ReportKind`'s own documentation
/// for why that distinction is in the report rather than only in a comment.
#[test]
fn a_program_that_reads_an_empty_stack_draws_and_reports() {
    let (mut device, adapter) = device();
    // Three pops off a stack of two, then `1.0 add`: the third pop and the addition both
    // read a value the stack has not got, and the sum is `0 + 1`. Admitted, because the
    // empty-stack value is a decision rather than a refusal — and reported, because it is
    // a decision ISO 32000-2 does not take.
    let program = device
        .upload_function(&[
            FnOp::Pop,
            FnOp::Pop,
            FnOp::Pop,
            FnOp::PushReal(1.0),
            FnOp::Add,
        ])
        .expect("admitted: the empty-stack read is a decision, not a refusal");
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(4.0, 4.0),
        )))
        .expect("upload");
    let scene = scene_of(
        program,
        outline,
        4.0,
        Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
        FnRange::Gray([0.0, 1.0]),
        None,
    );
    let frame = device
        .render(
            &scene,
            &Viewport::full(4, 4, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the frame is drawn");
    let reports = frame.reports().to_vec();
    let carried = reports
        .iter()
        .find(|report| report.kind == ReportKind::FunctionEmptyStackRead)
        .unwrap_or_else(|| panic!("{adapter}: no empty-stack report on the frame"));
    assert!(
        carried.detail.contains("reads an empty operand stack"),
        "{adapter}: {}",
        carried.detail
    );
    // The frame was drawn, and drawn is what it says: a report is not a refusal.
    assert_eq!(frame.reports().len(), 1);
    let pixels = frame.into_raster().unwrap().into_pixels();
    assert_eq!(
        pixel(&pixels, 4, 1, 1),
        [255, 255, 255, 255],
        "{adapter}: the empty-stack value is 0, so `0 1 add` is the grey level 1"
    );
}

/// A program that draws no report is a frame with no report: the diagnostic is a statement
/// about the program, so it must not appear for one that does not read an empty stack.
#[test]
fn a_program_that_reads_no_empty_stack_reports_nothing() {
    let (mut device, _adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(4.0, 4.0),
        )))
        .expect("upload");
    let scene = scene_of(
        program,
        outline,
        4.0,
        Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
        FnRange::Rgb([[0.0, 1.0]; 3]),
        None,
    );
    let frame = device
        .render(
            &scene,
            &Viewport::full(4, 4, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the frame is drawn");
    assert!(frame.reports().is_empty());
}

/// A released program is refused by name on the next frame, like every other resource
/// family — and the refusal is an `Err`, so nothing is drawn and nothing claims to be.
#[test]
fn a_released_program_is_refused_by_name() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(4.0, 4.0),
        )))
        .expect("upload");
    let scene = scene_of(
        program,
        outline,
        4.0,
        Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
        FnRange::Rgb([[0.0, 1.0]; 3]),
        None,
    );
    render(&mut device, &scene, 4, 4);
    device.release(program).expect("the program is resident");

    let refused = device.render(
        &scene,
        &Viewport::full(4, 4, Affine::IDENTITY),
        Target::Readback,
    );
    match refused {
        Err(RenderError::UnknownFunction { program: named }) => assert_eq!(named, program),
        other => panic!("{adapter}: expected a named refusal, got {other:?}"),
    }
}

/// A `Range` the program cannot fill is refused **before the quad is placed**, naming the
/// program and the pairing — the one question about a function paint that cannot be
/// answered until the program and the shading meet.
#[test]
fn a_range_the_program_cannot_fill_refuses_the_frame() {
    let (mut device, adapter) = device();
    // One value left, three components asked for.
    let program = device
        .upload_function(&[FnOp::Pop, FnOp::Pop, FnOp::PushReal(0.5)])
        .expect("admitted");
    let outline = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(4.0, 4.0),
        )))
        .expect("upload");
    let scene = scene_of(
        program,
        outline,
        4.0,
        Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
        FnRange::Rgb([[0.0, 1.0]; 3]),
        None,
    );
    match device.render(
        &scene,
        &Viewport::full(4, 4, Affine::IDENTITY),
        Target::Readback,
    ) {
        Err(RenderError::FunctionRangeRefused { program: named, .. }) => {
            assert_eq!(named, program);
        }
        other => panic!("{adapter}: expected a named Range refusal, got {other:?}"),
    }
}

/// The same scene twice is the same bytes on one adapter (§4.6), which is what says the
/// generated pipeline is reused rather than re-derived into something subtly different.
#[test]
fn two_frames_of_one_function_scene_are_identical() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let outline = device.upload_outline(&triangle(16.0)).expect("upload");
    let scene = scene_of(
        program,
        outline,
        16.0,
        Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
        FnRange::Rgb([[0.0, 1.0]; 3]),
        None,
    );
    let first = render(&mut device, &scene, 16, 16);
    let second = render(&mut device, &scene, 16, 16);
    assert_eq!(
        first, second,
        "{adapter}: a frame is a function of its inputs"
    );
}

/// **What a function fill costs, as counters rather than as a clock.**
///
/// `tests/archetypes.rs`'s discipline, applied where it fits: every `Counters` field is an
/// exact function of the scene and the viewport, so these compare by equality on any machine
/// and under any load. The fixture is *not* in that file, and deliberately — its archetypes
/// are page shapes measured over the caller's 995-page corpus (`doc/corpus-profile.md`),
/// which carries a "shading/mesh fills" row and no function-shading row at all, so an
/// archetype for this paint would be an invented page of exactly the kind that file's own
/// opening paragraph refuses.
///
/// What the numbers say: a rect-hinted fill takes the analytic path and places **no**
/// coverage tile, while the same paint over a triangle places exactly one — the two lanes
/// `the_coverage_path_paints_the_same_colours_as_the_rect_path` compares, priced.
#[test]
fn a_function_fill_costs_one_command_and_a_tile_only_when_it_is_rasterised() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let domain = Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0));
    let range = FnRange::Rgb([[0.0, 1.0]; 3]);

    let square = device
        .upload_outline(&rect_outline(Rect::new(
            Point::new(0.0, 0.0),
            Point::new(16.0, 16.0),
        )))
        .expect("upload");
    let scene = scene_of(program, square, 16.0, domain, range, None);
    let counters = device
        .render(
            &scene,
            &Viewport::full(16, 16, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the frame is drawn")
        .counters();
    assert_eq!(counters.commands, 1, "{adapter}");
    assert_eq!(counters.commands_culled, 0, "{adapter}");
    assert_eq!(
        counters.tiles, 0,
        "{adapter}: a rect-hinted function fill needs no scratch tile"
    );
    assert_eq!(
        counters.layer_textures, 0,
        "{adapter}: a flat page of one fill allocates no layer"
    );

    let triangle = device.upload_outline(&triangle(16.0)).expect("upload");
    let scene = scene_of(program, triangle, 16.0, domain, range, None);
    let counters = device
        .render(
            &scene,
            &Viewport::full(16, 16, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the frame is drawn")
        .counters();
    assert_eq!(counters.commands, 1, "{adapter}");
    assert_eq!(
        counters.tiles, 1,
        "{adapter}: a curve-bounded function fill is one coverage tile"
    );
}

/// **One program, one shader compile, however many frames draw it** (ADR 0053's cache).
///
/// A property rather than a duration: the first frame names the compile in its own
/// `Timings::phases` and the second names none, which reads the same on a loaded machine as
/// on an idle one. What it costs is measured elsewhere and is 6.3 ms cold for a
/// 482-instruction witness; what this asserts is that a page pays it once.
#[test]
fn the_generated_shader_is_compiled_once_and_then_reused() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let outline = device.upload_outline(&triangle(16.0)).expect("upload");
    let scene = scene_of(
        program,
        outline,
        16.0,
        Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
        FnRange::Rgb([[0.0, 1.0]; 3]),
        None,
    );
    let compiles = |device: &mut Device| -> usize {
        device
            .render(
                &scene,
                &Viewport::full(16, 16, Affine::IDENTITY),
                Target::Readback,
            )
            .expect("the frame is drawn")
            .timings()
            .phases
            .iter()
            .filter(|(name, _)| *name == "function shader compile (first use)")
            .count()
    };
    assert_eq!(
        compiles(&mut device),
        1,
        "{adapter}: the first frame that draws a program compiles its shader"
    );
    assert_eq!(
        compiles(&mut device),
        0,
        "{adapter}: every later frame finds it cached"
    );
}
