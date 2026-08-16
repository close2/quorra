//! §11.4.6's two stages asked for by name over a §7.10.5 program: `Compose::DestOut`, then
//! `Compose::Plus` (ADR 0025, ADR 0053).
//!
//! # Why this file exists
//!
//! `Style::of` maps both stages onto the function lane's generated pipelines, and
//! `pipeline::function` compiles them — and until this file **nothing drew one**
//! (`doc/notes-function-wiring.md` §4.5; `doc/notes-function-tests.md` §5 names the same
//! hole from the other side). `tests/function_knockout.rs` draws the pair only as the two
//! halves a *knockout group* runs together, and the builder refuses a staged mark inside
//! such a group, so the two constructions are disjoint: nothing a knockout fixture asserts
//! says a lone `DestOut` or a lone `Plus` reaches the right pipeline.
//!
//! # Where the expected values come from
//!
//! ISO 32000-2 §11.4.6 weights the replacement by the element's own source shape:
//!
//! > 𝛼gi = (1 − 𝑓si) × 𝛼gi−1 + 𝑓si × 𝛼t
//!
//! and ADR 0025's whole subject is that a caller writes that stage as two marks —
//! `P' = (1 − f) × P + S`, per channel and premultiplied, `f` the element's shape and `S`
//! its own premultiplied deposit. §11.6.4.2 makes shape geometry, so the *paint's* alpha is
//! not part of `f`; §11.4.7.2 is the clause that keeps the two quantities apart.
//!
//! For this paint one more clause joins them. §8.7.4.5.2:
//!
//! > Points within the shading's bounding box (BBox) that fall outside this transformed
//! > domain rectangle shall be painted with the shading's background colour (Background);
//! > if the shading dictionary has no Background entry, such points shall be left
//! > unpainted.
//!
//! An unpainted point is not part of the object's geometry, so it has no shape and a
//! `DestOut` staged over it must erase nothing there — which is the assertion that separates
//! this lane from every other one, because no other paint can decline to mark a pixel its
//! own quad covers.
//!
//! # The fixture, and why it is not an opaque one
//!
//! `doc/notes-function-tests.md` §1.4: for a source of alpha 1, §11.4.6's replacement and an
//! ordinary premultiplied over-composite are the same arithmetic, and a function paint is
//! opaque wherever it marks *inside its domain*. So the only construction that can tell the
//! staged pair from source-over on this paint is §8.7.4.5.2's `Background` at an alpha below
//! one, and that is what the first test paints outside its domain.

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
    Affine, BlendMode, Color, Compose, FillRule, FnOp, FnRange, FunctionId, OutlineId, Paint,
    Point, Rect, Scene, SceneBuilder, Segment,
};

mod common;

use common::clause::deviation_from_the_clause;

const SIZE: u32 = 64;

/// `x y 0.5` — the colour at a point is its own position, so a stage that drew the right
/// weight at the wrong pixel cannot pass.
const POSITION_IS_COLOUR: [FnOp; 1] = [FnOp::PushReal(0.5)];

/// The opaque page the stages are drawn onto — `P` in the clause's line.
const UNDER: Color = Color {
    r: 0.9,
    g: 0.2,
    b: 0.1,
    a: 1.0,
};

/// The device this file's assertions are made on, named so a failure says where: ADR 0053
/// promises **no** cross-adapter identity for this paint. `QUORRA_ADAPTER` picks another;
/// the suite's default is the software rasteriser, as everywhere else in this tree.
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

/// A triangle with two diagonal edges, so partially covered pixels exist: axis-aligned
/// rectangles would agree while being wrong (§4.1 of the brief). It is also what sends the
/// mark through the **rasterised coverage** lane rather than the rect-hinted one, so the two
/// staged passes have to read the same tile.
fn wedge(device: &mut Device) -> OutlineId {
    device
        .upload_outline(&[
            Segment::MoveTo(Point::new(10.0, 10.0)),
            Segment::LineTo(Point::new(54.0, 10.0)),
            Segment::LineTo(Point::new(10.0, 54.0)),
            Segment::Close,
        ])
        .unwrap()
}

/// The whole target as a closed rectangle: the **rect-hinted** lane, where the quad is the
/// shape's device rectangle rather than a scratch tile.
fn full_rect(device: &mut Device) -> OutlineId {
    let side = SIZE as f32;
    device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(side, 0.0)),
            Segment::LineTo(Point::new(side, side)),
            Segment::LineTo(Point::new(0.0, side)),
            Segment::Close,
        ])
        .unwrap()
}

/// The opaque page every stage below is drawn onto.
fn backdrop(builder: &mut SceneBuilder) {
    builder
        .rect(
            Rect::new(Point::new(0.0, 0.0), Point::new(SIZE as f32, SIZE as f32)),
            Affine::IDENTITY,
            UNDER,
            None,
            None,
        )
        .unwrap();
}

/// The left quarter of the unit square, so three quarters of a full-target shape falls
/// outside the transformed domain rectangle (§8.7.4.5.2).
fn left_quarter() -> Rect {
    Rect::new(Point::new(0.0, 0.0), Point::new(0.25, 1.0))
}

/// One function-painted mark, composed as asked. The `Matrix` maps the unit square onto the
/// whole target, so the value at a pixel is its own position (§10.7.4 for the centre).
fn function_mark(
    builder: &mut SceneBuilder,
    outline: OutlineId,
    program: FunctionId,
    background: Option<Color>,
    compose: Compose,
) {
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Function {
                program,
                domain: left_quarter(),
                matrix: Affine::scale(SIZE as f32, SIZE as f32),
                range: FnRange::Rgb([[0.0, 1.0]; 3]),
                background,
            },
            None,
            BlendMode::Normal,
            compose,
            None,
        )
        .expect("a valid function fill");
}

/// **The pair is §11.4.6's line over a function paint, and source-over is not.**
///
/// Every quantity in `P' = (1 − f) × P + S` is read from the device through the lane under
/// test rather than assumed:
///
/// - `f`, the element's **shape**, is the alpha of the same mark painted with a `Background`
///   of alpha **1** — the paint then marks every point of the wedge, so its alpha is exactly
///   the geometric coverage §11.6.4.2 calls shape. That this is also the shape of the
///   *half-opaque* element is ADR 0025's rule and is asserted on its own by
///   [`dest_out_over_a_function_paint_ignores_the_paints_own_opacity`].
/// - `S` is the element drawn onto transparency: its own premultiplied deposit.
/// - `P` is the opaque page before either stage.
///
/// The element's `Background` is alpha ½, so outside the transformed domain rectangle it has
/// shape 1 at opacity ½ — the only construction on this paint where the staged pair and an
/// ordinary over-composite disagree (`doc/notes-function-tests.md` §1.4). The control is
/// that same element as one `Compose::SrcOver` mark, measured against the same line, and it
/// **must miss it**: a fixture where the two readings agree holds nothing.
#[test]
fn the_staged_pair_over_a_function_paint_is_the_clause() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("`x y 0.5` is admitted");
    let outline = wedge(&mut device);
    // 0.2, 0.4 and 0.6 are exact in 8 bits (51, 102, 153); the alpha is the quantity that
    // makes the fixture discriminating and is stated as a fraction.
    let translucent = Color::new(0.2, 0.4, 0.6, 0.5);
    let opaque = Color::new(0.2, 0.4, 0.6, 1.0);

    let onto_transparency = |device: &mut Device, background: Color| {
        let mut builder = SceneBuilder::new();
        function_mark(
            &mut builder,
            outline,
            program,
            Some(background),
            Compose::SrcOver,
        );
        render(device, &builder.finish())
    };
    let shape = onto_transparency(&mut device, opaque);
    let deposit = onto_transparency(&mut device, translucent);

    let mut before = SceneBuilder::new();
    backdrop(&mut before);
    let before = render(&mut device, &before.finish());

    let mut staged_scene = SceneBuilder::new();
    backdrop(&mut staged_scene);
    function_mark(
        &mut staged_scene,
        outline,
        program,
        Some(translucent),
        Compose::DestOut,
    );
    function_mark(
        &mut staged_scene,
        outline,
        program,
        Some(translucent),
        Compose::Plus,
    );
    let staged = render(&mut device, &staged_scene.finish());

    let mut over_scene = SceneBuilder::new();
    backdrop(&mut over_scene);
    function_mark(
        &mut over_scene,
        outline,
        program,
        Some(translucent),
        Compose::SrcOver,
    );
    let over = render(&mut device, &over_scene.finish());

    let (worst_staged, partial) = deviation_from_the_clause(&before, &shape, &deposit, &staged);
    let (worst_over, _) = deviation_from_the_clause(&before, &shape, &deposit, &over);
    eprintln!("{adapter}: worst staged {worst_staged:.2}, worst source-over {worst_over:.2}");
    assert!(
        partial > 30,
        "{adapter}: the fixture must have partially covered pixels for this to mean \
         anything: {partial}"
    );
    assert!(
        worst_staged <= 3.0,
        "{adapter}: the pair asked for by name must be §11.4.6's line on this paint too; \
         worst premultiplied deviation {worst_staged}"
    );
    assert!(
        worst_over >= 16.0,
        "{adapter}: and one source-over mark must not be — it weights the page by \
         1 − shape × opacity where the clause weights it by 1 − shape, and a fixture where \
         the two agree holds nothing: {worst_over}"
    );
}

/// **A point §8.7.4.5.2 leaves unpainted has no shape, so a staged `DestOut` erases nothing
/// there.**
///
/// The domain is the left quarter of the target and the shading has no `Background`, so the
/// clause leaves three quarters of this full-target mark unpainted. §11.6.4.2 gives shape
/// from geometry and an unpainted point is not part of it, so `DestOut` — which ADR 0025
/// defines as weighting the backdrop by `1 − shape` — must take the page down to nothing on
/// the left and leave it **byte for byte** on the right.
///
/// This is `tests/function_knockout.rs`'s domain assertion asked of the *staged* pipeline
/// rather than of the knockout pair. The two are different selections of the same generated
/// module (`Style::of`), and a lane wired for one of them is not evidence about the other.
///
/// The mark is a rectangle, so this draws the **rect-hinted** placement where
/// [`the_staged_pair_over_a_function_paint_is_the_clause`] draws the rasterised-coverage one.
#[test]
fn dest_out_over_a_function_paint_erases_only_where_the_clause_paints() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let outline = full_rect(&mut device);

    let mut before = SceneBuilder::new();
    backdrop(&mut before);
    let before = render(&mut device, &before.finish());

    let mut builder = SceneBuilder::new();
    backdrop(&mut builder);
    function_mark(&mut builder, outline, program, None, Compose::DestOut);
    let erased = render(&mut device, &builder.finish());

    // x < 16 is inside the transformed domain rectangle; x >= 16 is outside it.
    for x in [0_u32, 8, 15] {
        assert_eq!(
            pixel(&erased, x, 32),
            [0, 0, 0, 0],
            "{adapter}: at x = {x} the paint marks at full shape, so `1 − shape` leaves \
             nothing of the page"
        );
    }
    for x in [16_u32, 32, 63] {
        assert_eq!(
            pixel(&erased, x, 32),
            pixel(&before, x, 32),
            "{adapter}: at x = {x} §8.7.4.5.2 leaves the point unpainted, so §11.6.4.2 \
             gives it no shape and `DestOut` has nothing to erase with"
        );
    }
}

/// **`DestOut` weights by shape and not by the paint's own opacity**, on the paint where the
/// two are easiest to confuse.
///
/// ADR 0025: `DestOut` weights by shape deliberately, because §11.6.4.2's shape is geometry
/// while §11.6.4.4's constant alpha and §11.6.4.3's soft mask are opacity — weighting by the
/// alpha would repeat the defect the operator exists to fix. For this paint the source of
/// opacity that has no equivalent anywhere else is §8.7.4.5.2's `Background`, whose alpha
/// changes what the mark *deposits* and must change nothing about what it *erases*.
///
/// So two erasures a quarter of an alpha apart must be the same frame, and — the half that
/// says the fixture erased at all — that frame must be empty, because with a `Background`
/// present the clause paints every point of the mark and its shape is therefore the whole
/// rectangle.
///
/// It is also the premise [`the_staged_pair_over_a_function_paint_is_the_clause`] rests on
/// when it reads `f` off an opaque-background mark and applies it to a half-opaque one.
#[test]
fn dest_out_over_a_function_paint_ignores_the_paints_own_opacity() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let outline = full_rect(&mut device);

    let erase_with = |device: &mut Device, alpha: f32| {
        let mut builder = SceneBuilder::new();
        backdrop(&mut builder);
        function_mark(
            &mut builder,
            outline,
            program,
            Some(Color::new(0.2, 0.4, 0.6, alpha)),
            Compose::DestOut,
        );
        render(device, &builder.finish())
    };

    let opaque = erase_with(&mut device, 1.0);
    let translucent = erase_with(&mut device, 0.25);
    assert_eq!(
        opaque, translucent,
        "{adapter}: §11.6.4.2's shape comes from geometry; a `Background`'s alpha is \
         opacity and may not change what is erased"
    );
    for (x, y) in [(2_u32, 2_u32), (32, 32), (63, 63)] {
        assert_eq!(
            pixel(&opaque, x, y),
            [0, 0, 0, 0],
            "{adapter}: with a `Background` the clause paints every point of the mark, so \
             its shape is the whole rectangle and `1 − shape` is 0 at ({x}, {y})"
        );
    }
}
