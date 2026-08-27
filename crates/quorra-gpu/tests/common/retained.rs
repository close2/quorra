//! The pages the `retained_*.rs` family draws, and the one call that asserts which encode
//! drew them.
//!
//! ADR 0048. The family is five files — replay, atlas overflow, the invalidation list,
//! refusals, and the handle's own surface — and every one of them needs the same two page
//! shapes and the same `render_retained` wrapper. One home, so that the day a page stops
//! reproducing what it is for, it stops doing so in one place.

use quorra_gpu::{Device, EncodeSource, Frame, Options, RetainedScene, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, GroupSpec, LineCap, LineJoin, OutlineId, Paint,
    Point, Scene, SceneBuilder, Segment, Stroke,
};

/// The target every page here is drawn into. Small on purpose: these tests assert which
/// encode drew a frame, not what a page costs.
pub const W: u32 = 220;
/// The target's height. See [`W`].
pub const H: u32 = 180;

/// The software adapter with `options`, warmed before it is handed back: `wait_until_warm`
/// puts every pipeline compile before the first frame rather than inside one.
pub fn device_with(options: &Options) -> Device {
    let device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..options.clone()
    })
    .expect("llvmpipe is present wherever this suite runs");
    device.wait_until_warm();
    device
}

/// [`device_with`] at the default options.
pub fn device() -> Device {
    device_with(&Options::default())
}

/// The viewport every page here is drawn through.
pub fn viewport() -> Viewport<'static> {
    Viewport::full(W, H, Affine::IDENTITY)
}

/// A closed curve of `segments` cubics, `side` across — the shape a letterform has for
/// costing purposes, as in `tests/archetypes.rs`.
pub fn blob(segments: u32, side: f32) -> Vec<Segment> {
    let radius = side * 0.5;
    let mut path = vec![Segment::MoveTo(Point::new(-radius, 0.0))];
    for step in 0..segments.max(3) {
        let angle = |i: u32| (i as f32) / (segments.max(3) as f32) * std::f32::consts::TAU;
        let at = |a: f32| Point::new(radius * a.cos(), radius * a.sin() * 1.3);
        let (a, b) = (at(angle(step)), at(angle(step + 1)));
        path.push(Segment::CubicTo {
            c1: Point::new(a.x + (b.x - a.x) * 0.35, a.y + (b.y - a.y) * 0.1),
            c2: Point::new(a.x + (b.x - a.x) * 0.65, a.y + (b.y - a.y) * 0.9),
            to: b,
        });
    }
    path.push(Segment::Close);
    path
}

/// Where the `index`th mark of a page of `side`-wide marks goes.
pub fn place(index: u32, side: f32) -> Affine {
    let step = side + 2.0;
    let columns = ((W as f32 - 8.0) / step).max(1.0) as u32;
    Affine::translate(
        6.0 + (index % columns) as f32 * step,
        10.0 + (index / columns) as f32 * step,
    )
}

/// A page of small repeated shapes: the glyph lane, most of it answered by the atlas.
///
/// One `side` for every outline, so every tile the atlas is asked for is the same size —
/// which is what lets `retained_invalidation.rs`'s `an_atlas_repack_re_encodes` reason
/// about the packer's capacity rather than hope.
pub fn text_page(
    device: &mut Device,
    placements: u32,
    distinct: u32,
    side: f32,
) -> (Scene, Vec<OutlineId>) {
    let outlines: Vec<OutlineId> = (0..distinct)
        .map(|_| device.upload_outline(&blob(12, side)).unwrap())
        .collect();
    let mut builder = SceneBuilder::new();
    for index in 0..placements {
        builder
            .fill(
                outlines[(index as usize) % outlines.len()],
                place(index, side + 2.0),
                FillRule::NonZero,
                Paint::Solid(Color::new(0.1, 0.12, 0.2, 1.0)),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
    }
    (builder.finish(), outlines)
}

/// The other lanes on one page: strokes, a **curve** clip (so every command under it
/// leaves a coverage tile on the frame's scratch sheet), and a blended group (so the
/// frame allocates layer textures and runs the compositor).
pub fn artwork_page(device: &mut Device) -> Scene {
    let shape = device.upload_outline(&blob(8, 26.0)).unwrap();
    let clip_shape = device.upload_outline(&blob(6, 120.0)).unwrap();
    let mut builder = SceneBuilder::new();
    let clip = builder
        .clip(
            clip_shape,
            Affine::translate(W as f32 * 0.5, H as f32 * 0.5),
            FillRule::NonZero,
            None,
        )
        .unwrap();
    for index in 0..12 {
        builder
            .stroke(
                shape,
                place(index, 30.0),
                Stroke {
                    width: 2.0,
                    adjust: false,
                    cap: LineCap::Round,
                    join: LineJoin::Miter,
                    miter_limit: 4.0,
                },
                Paint::Solid(Color::new(0.8, 0.2, 0.1, 1.0)),
                Some(clip),
                BlendMode::Normal,
                None,
            )
            .unwrap();
    }
    builder
        .group(
            GroupSpec {
                alpha: 0.7,
                blend: BlendMode::Multiply,
                clip: Some(clip),
                knockout: false,
                mask: None,
                isolated: true,
                compose: Compose::SrcOver,
            },
            |body| {
                for index in 0..6 {
                    body.fill(
                        shape,
                        place(index + 3, 30.0),
                        FillRule::NonZero,
                        Paint::Solid(Color::new(0.1, 0.5, 0.9, 1.0)),
                        Some(clip),
                        BlendMode::Normal,
                        Compose::SrcOver,
                        None,
                    )?;
                }
                Ok(())
            },
        )
        .unwrap();
    builder.finish()
}

/// Render through the handle and assert which encode drew it.
pub fn retained_frame(
    device: &mut Device,
    retained: &mut RetainedScene,
    viewport: &Viewport<'_>,
    expected: EncodeSource,
    why: &str,
) -> Frame {
    let frame = device
        .render_retained(retained, viewport, Target::Readback)
        .expect("the frame must draw");
    assert_eq!(frame.encode_source(), expected, "{why}");
    frame
}
