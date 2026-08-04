//! What a zoomed frame costs — the measurement behind command culling.
//!
//! A viewer zooms by handing the same page a larger transform and a viewport that
//! has not changed size. At 20× a 1191×1684 window shows about 1/400 of the page,
//! so a frame *should* get cheaper as the zoom rises: fewer glyphs are visible, and
//! every lane already clamps its geometry to the target before drawing.
//!
//! What this example prices is whether it does. ADR 0012 recorded that encode walks
//! the whole scene whatever is visible, and named command culling as the lever; the
//! numbers here are what that lever is worth, per zoom, per adapter.
//!
//! The scene is `floor.rs`'s dense page — 5 933 glyph-lane fills over 107 distinct
//! outlines — because it is the shape the brief's §0 says a document renderer must be
//! fast at, and because zoom is exactly where its premise (a few outlines repeated
//! many times) stops holding: past `MAX_GLYPH_DIM` every visible glyph leaves the
//! atlas for the coverage path and is rasterised again on every frame.
//!
//! Run: `cargo run --release -p quorra-gpu --example zoom`

// The f64→f32 casts build scene coordinates bounded by the page size; exact there.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::print_stdout
)]

use std::time::Duration;

use quorra_gpu::{Device, Options, Target, Viewport, wgpu};
use quorra_scene::{Affine, Color, Point, Scene, SceneBuilder, Segment};

/// A real window, which is what the zoom is relative to.
const WIDTH: u32 = 1191;
const HEIGHT: u32 = 1684;

/// 5 933 glyph-lane fills over 107 distinct outlines: `floor.rs`'s dense page, at
/// integer phases so the atlas helps as much as it can at 1×.
fn glyph_page(device: &mut Device) -> Scene {
    let mut outlines = Vec::new();
    for i in 0..107_u32 {
        let w = 6.0 + (i % 5) as f32;
        let h = 8.0 + (i % 7) as f32;
        outlines.push(
            device
                .upload_outline(&[
                    Segment::MoveTo(Point::new(0.3, 0.2)),
                    Segment::LineTo(Point::new(w, 0.0)),
                    Segment::CubicTo {
                        c1: Point::new(w + 1.0, h * 0.3),
                        c2: Point::new(w + 1.0, h * 0.7),
                        to: Point::new(w * 0.8, h),
                    },
                    Segment::LineTo(Point::new(0.0, h * 0.9)),
                    Segment::Close,
                ])
                .unwrap(),
        );
    }
    let mut builder = SceneBuilder::new();
    for i in 0..5_933_u32 {
        builder
            .fill(
                outlines[(i % 107) as usize],
                Affine::translate((i % 80) as f32 * 14.5, (i / 80) as f32 * 15.25),
                quorra_scene::FillRule::NonZero,
                quorra_scene::Paint::Solid(Color::new(0.1, 0.1, 0.1, 1.0)),
                None,
                quorra_scene::BlendMode::Normal,
                quorra_scene::Compose::SrcOver,
                None,
            )
            .unwrap();
    }
    builder.finish()
}

/// The viewport a viewer zoomed to `magnification` about the page's centre would ask
/// for: the window is the same size, the page is larger, and most of it is outside.
fn zoomed(magnification: f32) -> Affine {
    let (centre_x, centre_y) = (580.0, 565.0); // the dense page's middle
    Affine::translate(-centre_x, -centre_y)
        .then(Affine::scale(magnification, magnification))
        .then(Affine::translate(WIDTH as f32 / 2.0, HEIGHT as f32 / 2.0))
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

/// One frame at `magnification`, reported as (encode, execute, wall, culled,
/// segments).
fn frame(
    device: &mut Device,
    scene: &Scene,
    texture: &wgpu::Texture,
    magnification: f32,
) -> (Duration, Duration, Duration, u32, u32) {
    let viewport = Viewport::full(WIDTH, HEIGHT, zoomed(magnification));
    let started = std::time::Instant::now();
    let drawn = device
        .render(scene, &viewport, Target::Texture(texture))
        .expect("the dense page is within every budget");
    let wall = started.elapsed();
    let timings = drawn.timings();
    let counters = drawn.counters();
    (
        timings.encode,
        timings.execute,
        wall,
        counters.commands_culled,
        counters.segments,
    )
}

fn main() {
    let mut device = Device::headless(&Options::default()).expect("some adapter must exist");
    device.wait_until_warm();
    let scene = glyph_page(&mut device);
    let texture = device.wgpu().0.create_texture(&wgpu::TextureDescriptor {
        label: Some("zoom target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    println!(
        "dense glyph page at {WIDTH}x{HEIGHT} on {}",
        device.description()
    );
    println!("held at one magnification (fastest of five, after a warm-up frame)");
    println!("  zoom   encode      execute     wall      culled/5933  segments");
    for magnification in [1.0_f32, 4.0, 20.0, 100.0] {
        frame(&mut device, &scene, &texture, magnification);
        let mut best = (Duration::MAX, Duration::MAX, Duration::MAX);
        let mut counted = (0, 0);
        for _ in 0..5 {
            let (encode, execute, wall, culled, segments) =
                frame(&mut device, &scene, &texture, magnification);
            best = (best.0.min(encode), best.1.min(execute), best.2.min(wall));
            counted = (culled, segments);
        }
        println!(
            "  {magnification:>5.0}  {:>7.3} ms  {:>7.3} ms  {:>7.3} ms  {:>7}      {:>8}",
            milliseconds(best.0),
            milliseconds(best.1),
            milliseconds(best.2),
            counted.0,
            counted.1,
        );
    }

    // A zoom *gesture*, which is the case a cache cannot help: every frame carries a
    // different linear transform, so every glyph key is new and every tile is cold.
    // Worst of the sweep, not the fastest — a gesture is judged by its slowest frame.
    println!("sweeping 1x -> 20x over 24 frames (worst frame, every tile cold)");
    let mut worst = (Duration::ZERO, Duration::ZERO, Duration::ZERO);
    for step in 0..24_u32 {
        let magnification = 1.0 + (step as f32) * (19.0 / 23.0);
        let (encode, execute, wall, _, _) = frame(&mut device, &scene, &texture, magnification);
        worst = (worst.0.max(encode), worst.1.max(execute), worst.2.max(wall));
    }
    println!(
        "         {:>7.3} ms  {:>7.3} ms  {:>7.3} ms",
        milliseconds(worst.0),
        milliseconds(worst.1),
        milliseconds(worst.2),
    );
}
