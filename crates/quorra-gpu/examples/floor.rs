//! The §11.1 measurement: how much of a frame's fixed cost is the readback?
//!
//! The brief's §6.1 could not separate execution from readback — a bytes-per-second
//! estimate had to stand in — and ranked the whole design around the resulting
//! uncertainty. This example takes the measurement it asked for: the same viewport
//! rendered through `Target::Readback` and `Target::Texture`, phases split by
//! timestamp queries, fastest of ten after a warm-up, per adapter and per resolution.
//!
//! The scenes are the two book-ends of §6.1's table: one small rectangle (the floor —
//! what a target costs before the page's content matters) and a dense 5 933-rect page
//! (a dense text page's command count, rectangles standing in for glyphs until M4).
//!
//! Since M4/M5 the sweep also measures the glyph lanes for §11.3: a dense page of
//! maximal reuse (107 outlines, integer phases) against a page the atlas cannot help
//! (every fill at a fresh sub-pixel phase). The cold column is the first frame — the
//! one that rasterises tiles — and the warm column the fastest of ten after it.
//!
//! Run: `cargo run --release -p quorra-gpu --example floor`
//! The numbers land in `doc/PLAN.md`; RADV and llvmpipe both matter.

// The f64→f32 casts build scene coordinates bounded by the page size; exact there.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]

use std::time::{Duration, Instant};

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{Affine, Color, Point, Rect, Scene, SceneBuilder};

fn one_rect() -> Scene {
    let mut builder = SceneBuilder::new();
    builder
        .rect(
            Rect::new(Point::new(10.25, 10.5), Point::new(90.75, 60.125)),
            Affine::IDENTITY,
            Color::new(0.2, 0.4, 0.6, 1.0),
            None,
            None,
        )
        .unwrap();
    builder.finish()
}

/// 5 933 glyph-lane fills over 107 distinct outlines at integer phases: the caller's
/// dense page, maximal reuse.
fn glyph_page(device: &mut Device, unique_phases: bool) -> Scene {
    let mut outlines = Vec::new();
    for i in 0..107_u32 {
        let w = 6.0 + (i % 5) as f32;
        let h = 8.0 + (i % 7) as f32;
        let outline = device
            .upload_outline(&[
                quorra_scene::Segment::MoveTo(Point::new(0.3, 0.2)),
                quorra_scene::Segment::LineTo(Point::new(w, 0.0)),
                quorra_scene::Segment::CubicTo {
                    c1: Point::new(w + 1.0, h * 0.3),
                    c2: Point::new(w + 1.0, h * 0.7),
                    to: Point::new(w * 0.8, h),
                },
                quorra_scene::Segment::LineTo(Point::new(0.0, h * 0.9)),
                quorra_scene::Segment::Close,
            ])
            .unwrap();
        outlines.push(outline);
    }
    let mut builder = SceneBuilder::new();
    for i in 0..5_933_u32 {
        let phase = if unique_phases {
            (i as f32) * 0.061_3 % 1.0
        } else {
            0.0
        };
        builder
            .fill(
                outlines[(i % 107) as usize],
                Affine::translate((i % 80) as f32 * 14.5 + phase, (i / 80) as f32 * 15.25),
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

/// The uploaded resources one figure load needs: a 64×64 image, a two-stop ramp, a
/// device-anchored mesh, and the 180×120 rectangle the shadings fill.
fn figure_resources(
    device: &mut Device,
) -> (
    quorra_scene::ImageId,
    quorra_scene::RampId,
    quorra_scene::MeshId,
    quorra_scene::OutlineId,
) {
    use quorra_scene::{ImageSpec, MeshSpec, Segment, Stop};
    let texels: Vec<u8> = (0..64u32 * 64 * 4).map(|i| (i % 255) as u8).collect();
    let image = device
        .upload_image(&ImageSpec {
            width: 64,
            height: 64,
            data: std::sync::Arc::from(texels.as_slice()),
        })
        .unwrap();
    let ramp = device
        .upload_ramp(&[
            Stop {
                offset: 0.0,
                color: Color::new(0.9, 0.3, 0.1, 1.0),
            },
            Stop {
                offset: 1.0,
                color: Color::new(0.1, 0.3, 0.9, 1.0),
            },
        ])
        .unwrap();
    let mesh = device
        .upload_mesh(&MeshSpec {
            left: 700,
            top: 1300,
            image: ImageSpec {
                width: 64,
                height: 64,
                data: std::sync::Arc::from(texels.as_slice()),
            },
        })
        .unwrap();
    let shaded_rect = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(180.0, 0.0)),
            Segment::LineTo(Point::new(180.0, 120.0)),
            Segment::LineTo(Point::new(0.0, 120.0)),
            Segment::Close,
        ])
        .unwrap();
    (image, ramp, mesh, shaded_rect)
}

/// M7's measurement: the dense text page with a real page's worth of figures on
/// top — images, ramp sweeps and a mesh, each a per-op uniform quad (ADR 0011),
/// so this is the cost of "rare case" at a realistic count.
fn figure_page(device: &mut Device) -> Scene {
    use quorra_scene::{BlendMode, Compose, FillRule, ImageFilter, Paint, ShadingKind};
    let (image, ramp, mesh, shaded_rect) = figure_resources(device);

    let mut builder = SceneBuilder::new();
    for i in 0..5_933_u32 {
        let x = f64::from(i % 80).mul_add(14.5, 3.25) as f32;
        let y = f64::from(i / 80).mul_add(15.25, 4.5) as f32;
        builder
            .rect(
                Rect::new(Point::new(x, y), Point::new(x + 9.75, y + 11.5)),
                Affine::IDENTITY,
                Color::new(0.1, 0.1, 0.1, 1.0),
                None,
                None,
            )
            .unwrap();
    }
    for i in 0..8_u32 {
        let filter = if i % 4 == 0 {
            ImageFilter::Linear
        } else {
            ImageFilter::Nearest
        };
        let place = Affine {
            a: 200.0,
            b: 0.0,
            c: 0.0,
            d: 150.0,
            e: (i % 4) as f32 * 290.0 + 30.0,
            f: (i / 4) as f32 * 700.0 + 120.0,
        };
        builder
            .image(image, place, 1.0, filter, None, BlendMode::Normal, None)
            .unwrap();
    }
    for i in 0..6_u32 {
        let at = Affine::translate(
            (i % 3) as f32 * 350.0 + 60.0,
            (i / 3) as f32 * 500.0 + 900.0,
        );
        let kind = if i % 2 == 0 {
            ShadingKind::Axial {
                start: Point::new(0.0, 0.0),
                end: Point::new(180.0, 0.0),
                extend: (true, true),
            }
        } else {
            ShadingKind::Radial {
                start: Point::new(90.0, 60.0),
                start_radius: 0.0,
                end: Point::new(90.0, 60.0),
                end_radius: 90.0,
                extend: (true, true),
            }
        };
        builder
            .fill(
                shaded_rect,
                at,
                FillRule::NonZero,
                // The shading anchors through its own transform; `at` places both
                // the shape and the sweep identically here.
                Paint::Shading {
                    ramp,
                    kind,
                    transform: at,
                },
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
    }
    builder
        .fill(
            shaded_rect,
            Affine::translate(690.0, 1290.0),
            FillRule::NonZero,
            Paint::Mesh(mesh),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .unwrap();
    builder.finish()
}

fn dense_page() -> Scene {
    let mut builder = SceneBuilder::new();
    for i in 0..5_933_u32 {
        let x = f64::from(i % 80).mul_add(14.5, 3.25) as f32;
        let y = f64::from(i / 80).mul_add(15.25, 4.5) as f32;
        builder
            .rect(
                Rect::new(Point::new(x, y), Point::new(x + 9.75, y + 11.5)),
                Affine::IDENTITY,
                Color::new(0.1, 0.1, 0.1, 1.0),
                None,
                None,
            )
            .unwrap();
    }
    builder.finish()
}

struct Best {
    encode: Duration,
    upload: Duration,
    execute: Duration,
    readback: Duration,
    wall: Duration,
}

fn fastest_of(
    n: u32,
    device: &mut Device,
    scene: &Scene,
    viewport: &Viewport<'_>,
    readback: bool,
) -> Best {
    let mut best = Best {
        encode: Duration::MAX,
        upload: Duration::MAX,
        execute: Duration::MAX,
        readback: Duration::MAX,
        wall: Duration::MAX,
    };
    for _ in 0..n {
        let target_texture;
        let started = Instant::now();
        let frame = if readback {
            device.render(scene, viewport, Target::Readback).unwrap()
        } else {
            let (gpu, _) = device.wgpu();
            target_texture = gpu.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: viewport.width,
                    height: viewport.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            device
                .render(scene, viewport, Target::Texture(&target_texture))
                .unwrap()
        };
        let wall = started.elapsed();
        let timings = frame.timings();
        best.encode = best.encode.min(timings.encode);
        best.upload = best.upload.min(timings.upload);
        best.execute = best.execute.min(timings.execute);
        best.readback = best.readback.min(timings.readback);
        best.wall = best.wall.min(wall);
    }
    best
}

fn milliseconds(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// §6.5 / M8: the dense page patched through one caret-sized damage rect versus
/// redrawn whole, both onto a retained caller texture (ADR 0012's measurement).
fn measure_caret_blink(device: &mut Device) {
    // §6.5 / M8: the caret blink — the dense page patched through one 12×18
    // damage rect versus redrawn whole, both onto a retained caller texture.
    let scene = dense_page();
    let caret = [Rect::new(
        Point::new(400.0, 800.0),
        Point::new(412.0, 818.0),
    )];
    let (gpu, _) = device.wgpu();
    let retained = gpu.create_texture(&wgpu::TextureDescriptor {
        label: Some("floor retained"),
        size: wgpu::Extent3d {
            width: 1191,
            height: 1684,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    for (label, damage) in [("blink (full)", &[][..]), ("blink (damage)", &caret[..])] {
        let viewport = Viewport {
            width: 1191,
            height: 1684,
            transform: Affine::IDENTITY,
            damage,
        };
        let mut best = Best {
            encode: Duration::MAX,
            upload: Duration::MAX,
            execute: Duration::MAX,
            readback: Duration::MAX,
            wall: Duration::MAX,
        };
        for _ in 0..11 {
            let started = Instant::now();
            let frame = device
                .render(&scene, &viewport, Target::Texture(&retained))
                .unwrap();
            let wall = started.elapsed();
            let timings = frame.timings();
            best.encode = best.encode.min(timings.encode);
            best.execute = best.execute.min(timings.execute);
            best.wall = best.wall.min(wall);
        }
        println!(
            "{:<16} {:>6}x{:<4} {:>7} | {:>8} {:>7.3} {:>7} {:>8.3} {:>8} {:>8.3}",
            label,
            1191,
            1684,
            "tex",
            "",
            milliseconds(best.encode),
            "",
            milliseconds(best.execute),
            "",
            milliseconds(best.wall),
        );
    }
}

fn main() {
    let sizes: [(u32, u32); 3] = [(596, 842), (1191, 1684), (2382, 3368)];
    let scenes = [("1 rect", one_rect()), ("5933 rects", dense_page())];

    for adapter in ["llvmpipe", "RADV"] {
        let Ok(mut device) = Device::headless(&Options {
            adapter: Some(adapter.into()),
            ..Options::default()
        }) else {
            eprintln!("adapter '{adapter}' unavailable; skipped");
            continue;
        };
        device.wait_until_warm();
        let startup = device.startup();
        println!("\n== {} ==", device.description());
        println!(
            "startup: adapter {:.2} ms, device {:.2} ms, warm pipelines {:.2} ms",
            milliseconds(startup.adapter_enumeration),
            milliseconds(startup.device_creation),
            startup.pipeline_compilation.map_or(f64::NAN, milliseconds),
        );
        println!(
            "{:<16} {:>11} {:>7} | {:>8} {:>8} {:>8} {:>9} {:>9} {:>9}",
            "scene",
            "target",
            "kind",
            "cold-enc",
            "encode",
            "upload",
            "execute",
            "readback",
            "frame"
        );
        for (label, scene) in &scenes {
            for (w, h) in sizes {
                let viewport = Viewport::full(w, h, Affine::IDENTITY);
                for (kind, readback) in [("read", true), ("tex", false)] {
                    // The discarded first frame is the cold one — for glyph scenes it
                    // is where tiles rasterise, which is §11.3's cost.
                    let cold = fastest_of(1, &mut device, scene, &viewport, readback);
                    let best = fastest_of(10, &mut device, scene, &viewport, readback);
                    println!(
                        "{:<16} {:>6}x{:<4} {:>7} | {:>7.3} {:>7.3} {:>7.3} {:>8.3} {:>8.3} {:>8.3}",
                        label,
                        w,
                        h,
                        kind,
                        milliseconds(cold.encode),
                        milliseconds(best.encode),
                        milliseconds(best.upload),
                        milliseconds(best.execute),
                        milliseconds(best.readback),
                        milliseconds(best.wall),
                    );
                }
            }
        }

        // §11.3: the glyph lane at the two book-ends of reuse, at window scale;
        // M7: the same dense page carrying a realistic figure load.
        let glyph_scenes = [
            ("glyphs (reuse)", glyph_page(&mut device, false)),
            ("glyphs (unique)", glyph_page(&mut device, true)),
            ("rects+figures", figure_page(&mut device)),
        ];
        for (label, scene) in &glyph_scenes {
            let viewport = Viewport::full(1191, 1684, Affine::IDENTITY);
            let cold = fastest_of(1, &mut device, scene, &viewport, false);
            let best = fastest_of(10, &mut device, scene, &viewport, false);
            println!(
                "{:<16} {:>6}x{:<4} {:>7} | {:>7.3} {:>7.3} {:>7.3} {:>8.3} {:>8.3} {:>8.3}",
                label,
                1191,
                1684,
                "tex",
                milliseconds(cold.encode),
                milliseconds(best.encode),
                milliseconds(best.upload),
                milliseconds(best.execute),
                milliseconds(best.readback),
                milliseconds(best.wall),
            );
        }

        measure_caret_blink(&mut device);

        // §11.5's input: what holding a scene costs, for the dozen-pages verdict.
        println!(
            "scene memory: dense page {} bytes, figure page {} bytes",
            dense_page().cost().retained_bytes,
            figure_page(&mut device).cost().retained_bytes,
        );
    }
}
