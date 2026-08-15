//! A §7.10.5 program drawn through a **retained** frame (ADR 0048, ADR 0053).
//!
//! `tests/retained_frame.rs` enumerates ADR 0045's invalidation list over the glyph, the
//! coverage and the compositor lanes. It draws no function paint, and
//! `doc/notes-function-wiring.md` §6 says so: *"A `Paint::Function` op replays like any
//! other — it is an `Op` in a `LayerPlan` and carries no device handle beyond a raw id —
//! but no test in `retained_frame.rs` draws one."* A replayed encode is a machine for
//! producing a plausible-looking wrong page, which is principle 6's worst outcome, so
//! "like any other" is a claim to check rather than to assume.
//!
//! # What is different about this paint, and therefore what is tested
//!
//! Two things the other lanes do not have:
//!
//! - **A `FunctionId` in an op, and a compiled shader behind it.** The op carries the raw
//!   id; the pipeline is cached by the *program's content hash*, and released with the last
//!   resident program naming it (`PipelineStore::forget_program`). So a release has two
//!   effects, and a replay must survive neither of them: the id stops resolving, and the
//!   pipeline the encode's frame compiled is gone.
//! - **`EncodeKey::resource_generation`**, which is in the key for exactly this. It counts
//!   *releases*, so the tests below assert it in both directions: an upload must not
//!   invalidate an encode (the id space never reissues, so no retained id can come to mean
//!   other bytes) and a release must.
//!
//! # Assertions, never durations
//!
//! `Frame::encode_source` says which encode drew the pixels, and the compile is counted by
//! name out of `Timings::phases`. Nothing here reads a clock: this machine reached load 66
//! yesterday, and "the second frame was faster" says nothing about *which* encode drew it.

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

use quorra_gpu::{
    Device, EncodeSource, Frame, Options, RenderError, RetainedScene, Target, Viewport,
};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, FnOp, FnRange, FunctionId, OutlineId, Paint,
    Point, Rect, Scene, SceneBuilder, Segment,
};

const SIZE: u32 = 48;

/// `x y 0.5` — a function of both inputs, so a replay that drew the quad at the wrong
/// place would produce different bytes rather than the same flat colour.
const POSITION_IS_COLOUR: [FnOp; 1] = [FnOp::PushReal(0.5)];

/// `x y 0.25` — the same shape of program with one literal changed, so it is a *distinct*
/// program with a distinct content hash and a distinct generated shader.
const A_SECOND_PROGRAM: [FnOp; 1] = [FnOp::PushReal(0.25)];

/// The device this file's assertions are made on, named so a failure says where (ADR 0053
/// promises no cross-adapter identity for this paint).
fn device() -> (Device, String) {
    let requested = std::env::var("QUORRA_ADAPTER").unwrap_or_else(|_| "llvmpipe".into());
    let device = Device::headless(&Options {
        adapter: Some(requested),
        ..Options::default()
    })
    .expect("the requested adapter is present");
    device.wait_until_warm();
    let name = device.description().to_string();
    (device, name)
}

fn viewport() -> Viewport<'static> {
    Viewport::full(SIZE, SIZE, Affine::IDENTITY)
}

fn rect_outline(device: &mut Device, rect: Rect) -> OutlineId {
    device
        .upload_outline(&[
            Segment::MoveTo(rect.min),
            Segment::LineTo(Point::new(rect.max.x, rect.min.y)),
            Segment::LineTo(rect.max),
            Segment::LineTo(Point::new(rect.min.x, rect.max.y)),
            Segment::Close,
        ])
        .expect("upload")
}

/// A closed triangle: the fill takes the rasterised-coverage lane, so the retained encode
/// carries a scratch sheet as well as an op.
fn triangle(device: &mut Device, side: f32) -> OutlineId {
    device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(side, 0.0)),
            Segment::LineTo(Point::new(0.0, side)),
            Segment::Close,
        ])
        .expect("upload")
}

fn function_fill(
    builder: &mut SceneBuilder,
    outline: OutlineId,
    program: FunctionId,
    transform: Affine,
) {
    builder
        .fill(
            outline,
            transform,
            FillRule::NonZero,
            Paint::Function {
                program,
                domain: Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
                // §8.7.4.5.2's Matrix: the unit square onto the target, so a pixel's colour
                // is its own position and a misplaced quad is visible in the bytes.
                matrix: Affine::scale(SIZE as f32, SIZE as f32),
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

/// A page that draws the program through **both** placements of the lane: a rect-hinted
/// fill, which places its quad from the shape's device rectangle, and a curve-bounded one,
/// which places it on a coverage tile in the frame's scratch sheet. A replay that dropped
/// the sheet would pass the first and fail the second.
fn function_page(device: &mut Device, program: FunctionId) -> Scene {
    let square = rect_outline(
        device,
        Rect::new(Point::new(0.0, 0.0), Point::new(20.0, 20.0)),
    );
    let wedge = triangle(device, 20.0);
    let mut builder = SceneBuilder::new();
    function_fill(&mut builder, square, program, Affine::IDENTITY);
    function_fill(&mut builder, wedge, program, Affine::translate(24.0, 24.0));
    // A solid fill beside them, so the page is not one lane wide.
    builder
        .rect(
            Rect::new(Point::new(0.0, 30.0), Point::new(20.0, 40.0)),
            Affine::IDENTITY,
            Color::new(0.1, 0.6, 0.3, 1.0),
            None,
            None,
        )
        .unwrap();
    builder.finish()
}

fn pixels(frame: Frame) -> Vec<u8> {
    frame.into_raster().unwrap().into_pixels()
}

/// Render through the handle and assert which encode drew it.
fn retained_frame(
    device: &mut Device,
    retained: &mut RetainedScene,
    expected: EncodeSource,
    why: &str,
) -> Frame {
    let frame = device
        .render_retained(retained, &viewport(), Target::Readback)
        .expect("the frame must draw");
    assert_eq!(frame.encode_source(), expected, "{why}");
    frame
}

/// How many generated shaders this frame compiled, by the phase the lane names itself with.
///
/// A count of distinct compiles rather than a duration: what it costs is a function of the
/// program's length and is measured elsewhere, and a wall clock on this machine is worth
/// nothing. `tests/function_lane.rs` reads the same phase for the same reason.
fn compiles(frame: &Frame) -> usize {
    frame
        .timings()
        .phases
        .iter()
        .filter(|(name, _)| *name == "function shader compile (first use)")
        .count()
}

/// **A replayed function page is the page that was encoded**, byte for byte, on both of the
/// lane's placements at once.
///
/// Three renderings of one scene at one viewport — immediate, retained-and-encoded,
/// retained-and-replayed — and the fourth frame proves a replay does not consume what it
/// replays.
#[test]
fn a_replayed_function_page_is_the_page_that_was_encoded() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("`x y 0.5` is admitted");
    let scene = function_page(&mut device, program);

    let immediate = device
        .render(&scene, &viewport(), Target::Readback)
        .expect("the frame must draw");
    assert_eq!(
        immediate.encode_source(),
        EncodeSource::Encoded,
        "{adapter}: `render` retains nothing and always encodes"
    );
    let immediate = pixels(immediate);

    let mut retained = RetainedScene::new(scene);
    let first = retained_frame(
        &mut device,
        &mut retained,
        EncodeSource::Encoded,
        "the first frame of a handle has nothing to replay",
    );
    let second = retained_frame(
        &mut device,
        &mut retained,
        EncodeSource::Replayed,
        "nothing an encode reads changed",
    );
    let third = retained_frame(
        &mut device,
        &mut retained,
        EncodeSource::Replayed,
        "a replay does not consume what it replays",
    );

    assert_eq!(
        pixels(first),
        immediate,
        "{adapter}: the retained path's own first frame must equal an immediate one"
    );
    assert_eq!(
        pixels(second),
        immediate,
        "{adapter}: a replayed function page must equal the page that was encoded, byte \
         for byte"
    );
    assert_eq!(
        pixels(third),
        immediate,
        "{adapter}: and the frame after it"
    );
}

/// **A replay compiles nothing**, because the generated pipeline is the device's and not
/// the encode's.
///
/// The first frame of a handle names the compile in its own `Timings::phases`; every later
/// one names none, replayed or re-encoded. That is the property ADR 0053's cache promises,
/// asserted where a retained encode could have hidden it — a replay that skipped phase 2's
/// pipeline lookup would report the same zero for a different reason, which is why the
/// byte identity above is a separate test rather than an afterthought here.
#[test]
fn a_replayed_frame_compiles_no_shader() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let scene = function_page(&mut device, program);
    let mut retained = RetainedScene::new(scene);

    let first = retained_frame(
        &mut device,
        &mut retained,
        EncodeSource::Encoded,
        "first frame",
    );
    assert_eq!(
        compiles(&first),
        1,
        "{adapter}: two placements of one program are one generated shader"
    );
    let second = retained_frame(
        &mut device,
        &mut retained,
        EncodeSource::Replayed,
        "unchanged",
    );
    assert_eq!(compiles(&second), 0, "{adapter}: and it is compiled once");
}

/// **A second distinct program is a second shader**, and a second *upload of the same
/// program* is not.
///
/// The pipeline cache is keyed by the program's content hash (ADR 0053), so what a page
/// pays is one compile per distinct program however many ids name it — which is exactly
/// what `Scene::cost`'s `function_programs` counts. Both halves are asserted in one frame,
/// because the interesting failure is a key that is too coarse, and a test that only ever
/// uploaded distinct programs could not see it.
#[test]
fn a_second_distinct_program_compiles_a_second_shader() {
    let (mut device, adapter) = device();
    let first = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let second = device.upload_function(&A_SECOND_PROGRAM).expect("admitted");
    // A third id over the *same* instructions as the first: two identifiers, one content
    // hash, and therefore one generated shader.
    let twin = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    assert_ne!(first, twin, "each upload mints its own identifier");

    let outline = rect_outline(
        &mut device,
        Rect::new(Point::new(0.0, 0.0), Point::new(12.0, 12.0)),
    );
    let mut builder = SceneBuilder::new();
    for (index, program) in [first, second, twin].into_iter().enumerate() {
        function_fill(
            &mut builder,
            outline,
            program,
            Affine::translate(index as f32 * 14.0, 0.0),
        );
    }
    let mut retained = RetainedScene::new(builder.finish());

    let frame = retained_frame(
        &mut device,
        &mut retained,
        EncodeSource::Encoded,
        "first frame",
    );
    assert_eq!(
        compiles(&frame),
        2,
        "{adapter}: three placements, two distinct programs, two generated shaders — the \
         cache keys on what the program is, not on which id named it"
    );
    let replayed = retained_frame(
        &mut device,
        &mut retained,
        EncodeSource::Replayed,
        "unchanged",
    );
    assert_eq!(compiles(&replayed), 0, "{adapter}");
}

/// **An upload replays and a release re-encodes** — `EncodeKey::resource_generation`, in
/// both of its directions, on a program neither frame draws.
///
/// The field's own documentation states the asymmetry: an upload cannot invalidate anything
/// *while ids remain*, because `ResourceStore::allocate_id` never reissues one, so a
/// retained encode's ids cannot come to name other bytes; a release can, and is therefore
/// the operation that moves the counter. Both halves are held here with a program the
/// retained scene never mentions, so what is observed is the counter rather than the scene:
/// if a function release did not move the generation, the third frame below would replay.
#[test]
fn an_unrelated_upload_replays_and_an_unrelated_release_re_encodes() {
    // No adapter in the messages: every assertion here is about which encode drew the
    // frame, which `EncodeKey` decides before any adapter is asked anything.
    let (mut device, _adapter) = device();
    let drawn = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let scene = function_page(&mut device, drawn);
    let mut retained = RetainedScene::new(scene);
    retained_frame(
        &mut device,
        &mut retained,
        EncodeSource::Encoded,
        "first frame",
    );
    retained_frame(
        &mut device,
        &mut retained,
        EncodeSource::Replayed,
        "unchanged",
    );

    let spare = device.upload_function(&A_SECOND_PROGRAM).expect("admitted");
    retained_frame(
        &mut device,
        &mut retained,
        EncodeSource::Replayed,
        "an upload mints a new id and moves no existing one: nothing the encode names \
         changed meaning",
    );

    device.release(spare).expect("the program is resident");
    retained_frame(
        &mut device,
        &mut retained,
        EncodeSource::Encoded,
        "a release moves the resource generation, and one counter serves all five id \
         spaces — a program's release must move it like any other",
    );
}

/// **Releasing the program the encode names does not let a stale encode draw.**
///
/// The strongest of these, and principle 6 in one assertion. The retained encode carries
/// this program's raw id; after the release the id names nothing, and a replay of that
/// encode would be a frame drawing a resource the device no longer has. It re-encodes
/// instead, and the fresh encode refuses by name — which is what `Device::render` over the
/// same scene does, so the retained path adds no way to produce a page immediate mode
/// could not.
///
/// The refusal leaves the handle holding nothing, and is identical on every later attempt.
#[test]
fn a_released_program_re_encodes_and_the_refusal_stands() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let scene = function_page(&mut device, program);
    let mut retained = RetainedScene::new(scene);
    retained_frame(
        &mut device,
        &mut retained,
        EncodeSource::Encoded,
        "first frame",
    );
    retained_frame(
        &mut device,
        &mut retained,
        EncodeSource::Replayed,
        "unchanged",
    );

    device.release(program).expect("the program is resident");
    for attempt in 0..3 {
        let refused = device.render_retained(&mut retained, &viewport(), Target::Readback);
        assert!(
            matches!(refused, Err(RenderError::UnknownFunction { program: named }) if named == program),
            "{adapter}: attempt {attempt}: a frame painting a released program must be \
             refused by name, not drawn from a stale encode: {refused:?}"
        );
        assert!(
            !retained.holds_encode(),
            "{adapter}: attempt {attempt}: the key that was held did not match, so what it \
             held was stale and nothing is retained"
        );
    }
}

/// **Re-uploading the same instructions does not revive the dead encode.**
///
/// The sequel to the test above, and the one the id space's rule is for: the new upload has
/// the *same* content hash — so the pipeline cache would answer for it — but a **different**
/// identifier, because `ResourceStore::allocate_id` never reissues one. The retained scene
/// still names the released id, so it is still refused; only a scene naming the new id
/// draws, and when it does it draws the same page as before.
///
/// It also pays for the shader again: releasing the last resident program with a hash drops
/// what it compiled (`PipelineStore::forget_program`), so the compile is a first use once
/// more. That is the observable half of the release having reached the pipeline store, and
/// a release that leaked the pipeline would report zero here.
#[test]
fn a_re_uploaded_program_is_a_new_id_and_draws_the_same_page() {
    let (mut device, adapter) = device();
    let program = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("admitted");
    let scene = function_page(&mut device, program);
    let mut retained = RetainedScene::new(scene);
    let drawn = pixels(retained_frame(
        &mut device,
        &mut retained,
        EncodeSource::Encoded,
        "first frame",
    ));

    device.release(program).expect("the program is resident");
    let again = device
        .upload_function(&POSITION_IS_COLOUR)
        .expect("the same instructions are admitted again");
    assert_ne!(
        again, program,
        "{adapter}: an identifier is never reissued, which is what lets an upload leave \
         every retained encode valid"
    );
    let refused = device.render_retained(&mut retained, &viewport(), Target::Readback);
    assert!(
        matches!(refused, Err(RenderError::UnknownFunction { .. })),
        "{adapter}: the handle still names the released id, and a program with the same \
         instructions is not that program: {refused:?}"
    );

    retained.set_scene(function_page(&mut device, again));
    let frame = retained_frame(
        &mut device,
        &mut retained,
        EncodeSource::Encoded,
        "a new scene is a new encode",
    );
    assert_eq!(
        compiles(&frame),
        1,
        "{adapter}: the release dropped the pipelines its hash keyed, so the shader is a \
         first use again"
    );
    assert_eq!(
        pixels(frame),
        drawn,
        "{adapter}: the same instructions under the same placement are the same page"
    );
    retained_frame(
        &mut device,
        &mut retained,
        EncodeSource::Replayed,
        "and the new encode replays like any other",
    );
}
