//! What `encode: geometry` costs, and what more than one thread does to it.
//!
//! The instrument for the caller's `doc/QUORRA_ENCODE_THREADS.md`: four page shapes,
//! each encoded at several values of [`Options::encode_threads`], round-robin, minima,
//! with the load average printed beside every number.
//!
//! ```text
//! cargo run --release -p quorra-gpu --example encode_threads \
//!     [-- <adapter substring> [rounds] [thread counts, comma separated]]
//! ```
//!
//! # How it measures, and why each choice
//!
//! - **Every sample is a cold frame on a fresh device.** The page this exists for fills
//!   the glyph atlas on its first frame and reads it on every frame after — the caller
//!   measured 406 ms of geometry and then 1.7 ms for the same view a second time — so a
//!   steady state of twenty frames would measure the tile cache and call it the
//!   rasteriser. A sample is therefore `Device::headless` + a scene + one frame, and only
//!   the frame is in the span.
//! - **Round-robin over the thread counts, minima reported.** This machine is somebody's
//!   desktop and its load average is not a constant (`doc/HANDOVER.md`); rotating the
//!   order each round puts the same drift on every configuration.
//! - **The counters are printed with the clocks**, and they are exact functions of the
//!   scene. Identical counters across thread counts is the claim this binary can make
//!   without believing a clock at all; the durations beside them are the claim it can
//!   only make loosely.
//! - **Headless into a texture created once per device**: no surface, no vsync, and no
//!   readback, since a copy-out would be the largest cost in a frame and it is not the
//!   subject.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use std::time::{Duration, Instant};

use quorra_gpu::{Counters, Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, ClipId, Color, Compose, FillRule, GroupSpec, LineCap, LineJoin, OutlineId,
    Paint, Point, Scene, SceneBuilder, Segment, Stroke,
};

/// One page shape, in the fields `tests/archetypes.rs` states its archetypes with. The
/// geometry that realises them is this file's own, deliberately: an example cannot reach
/// a test's module, and the counters printed below are what says the two agree.
struct Shape {
    name: &'static str,
    width: u32,
    height: u32,
    commands: u32,
    distinct: u32,
    segments: u32,
    side: f32,
    strokes: u32,
    clips: u32,
    clipped: u32,
    groups: u32,
    blended_groups: u32,
}

/// **The caller's page**, at the size and shape their trace measured: one geological
/// cross-section, 58 009 commands of which six are strokes, 51.9 path segments each, no
/// text, no images, no groups and not one clip — 3.0 M segments over a 900 × 1100 window
/// where a mark is about three device pixels across.
const DRAWING: Shape = Shape {
    name: "drawing",
    width: 900,
    height: 1100,
    commands: 58_009,
    distinct: 58_009,
    segments: 52,
    side: 3.0,
    strokes: 6,
    clips: 0,
    clipped: 0,
    groups: 0,
    blended_groups: 0,
};

/// `tests/archetypes.rs`'s artwork row: the corpus's p99 clip shape, and the archetype
/// `doc/PLAN.md` carries a geometry number for. Its 600 clipped marks are the case this
/// round does **not** divide, which is why it is here.
const ARTWORK: Shape = Shape {
    name: "artwork",
    width: 1191,
    height: 1684,
    commands: 900,
    distinct: 300,
    segments: 24,
    side: 60.0,
    strokes: 405,
    clips: 185,
    clipped: 600,
    groups: 8,
    blended_groups: 4,
};

/// `tests/archetypes.rs`'s dense-text row: the shape the glyph atlas exists for, where
/// five placements share every rasterisation.
const DENSE_TEXT: Shape = Shape {
    name: "dense text",
    width: 1191,
    height: 1684,
    commands: 4_320,
    distinct: 818,
    segments: 12,
    side: 11.0,
    strokes: 0,
    clips: 0,
    clipped: 0,
    groups: 0,
    blended_groups: 0,
};

/// `tests/archetypes.rs`'s median row: twelve marks and ninety-six segments, which is
/// most of a corpus. The floor's evidence — a page this size must not pay for the lane
/// the page above it wanted (the caller's §4, their ADR 0228).
const MEDIAN_PAGE: Shape = Shape {
    name: "median page",
    width: 1191,
    height: 1684,
    commands: 12,
    distinct: 9,
    segments: 8,
    side: 11.0,
    strokes: 0,
    clips: 0,
    clipped: 0,
    groups: 0,
    blended_groups: 0,
};

const SHAPES: [&Shape; 4] = [&DRAWING, &ARTWORK, &DENSE_TEXT, &MEDIAN_PAGE];

/// A closed curve of `segments` cubics about the origin, `side` across.
fn outline_of(segments: u32, side: f32) -> Vec<Segment> {
    let radius = side * 0.5;
    let mut path = vec![Segment::MoveTo(Point::new(-radius, 0.0))];
    let steps = segments.max(3);
    for step in 0..steps {
        let from = (step as f32) / (steps as f32) * std::f32::consts::TAU;
        let to = ((step + 1) as f32) / (steps as f32) * std::f32::consts::TAU;
        let point = |angle: f32| Point::new(radius * angle.cos(), radius * angle.sin() * 1.3);
        let (a, b) = (point(from), point(to));
        path.push(Segment::CubicTo {
            c1: Point::new(a.x + (b.x - a.x) * 0.35, a.y + (b.y - a.y) * 0.1),
            c2: Point::new(a.x + (b.x - a.x) * 0.65, a.y + (b.y - a.y) * 0.9),
            to: b,
        });
    }
    path.push(Segment::Close);
    path
}

fn position(shape: &Shape, index: u32, side: f32) -> Affine {
    let step = side + 3.5;
    let columns = ((shape.width as f32 - 16.0) / step).max(1.0) as u32;
    let x = 8.0 + (index % columns) as f32 * step + side * 0.5;
    let y = 12.0 + (index / columns) as f32 * (side + 4.25) + side * 0.5;
    Affine::translate(x, y % (shape.height as f32 - 24.0))
}

fn emit(
    builder: &mut SceneBuilder,
    shape: &Shape,
    outlines: &[OutlineId],
    clips: &[ClipId],
    i: u32,
) {
    let outline = outlines[(i as usize) % outlines.len()];
    let clip = (i < shape.clipped && !clips.is_empty()).then(|| clips[(i as usize) % clips.len()]);
    let ink = Color::new(0.12, 0.13, 0.16, 1.0);
    let at = position(shape, i, shape.side);
    if i < shape.strokes {
        builder
            .stroke(
                outline,
                at,
                Stroke {
                    width: 1.5,
                    cap: LineCap::Butt,
                    join: LineJoin::Miter,
                    miter_limit: 4.0,
                },
                Paint::Solid(ink),
                clip,
                BlendMode::Normal,
                None,
            )
            .unwrap();
    } else {
        builder
            .fill(
                outline,
                at,
                FillRule::NonZero,
                Paint::Solid(ink),
                clip,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
    }
}

fn build(device: &mut Device, shape: &Shape) -> Scene {
    let outlines: Vec<OutlineId> = (0..shape.distinct.max(1))
        .map(|i| {
            // A different side per outline is what makes them distinct shapes rather
            // than one shape uploaded many times: the caller's page has 58 003 of those.
            let side = shape.side * (1.0 + (i % 5) as f32 * 0.05);
            device
                .upload_outline(&outline_of(shape.segments, side))
                .unwrap()
        })
        .collect();
    let mut builder = SceneBuilder::new();
    let clips: Vec<ClipId> = (0..shape.clips)
        .map(|i| {
            let outline = outlines[(i as usize) % outlines.len()];
            builder
                .clip(
                    outline,
                    position(shape, i, shape.side * 6.0),
                    FillRule::NonZero,
                    None,
                )
                .unwrap()
        })
        .collect();
    let per_group = (shape.commands / 4)
        .checked_div(shape.groups)
        .map_or(0, |per| per.max(1));
    let grouped = per_group * shape.groups;
    for group in 0..shape.groups {
        let spec = GroupSpec {
            alpha: 0.8,
            blend: if group < shape.blended_groups {
                BlendMode::Multiply
            } else {
                BlendMode::Normal
            },
            clip: None,
            knockout: false,
            mask: None,
            isolated: true,
            compose: Compose::SrcOver,
        };
        builder
            .group(spec, |body| {
                for step in 0..per_group {
                    emit(body, shape, &outlines, &clips, group * per_group + step);
                }
                Ok(())
            })
            .unwrap();
    }
    for index in grouped..shape.commands {
        emit(&mut builder, shape, &outlines, &clips, index);
    }
    builder.finish()
}

fn target_texture(device: &Device, shape: &Shape) -> wgpu::Texture {
    let (gpu, _) = device.wgpu();
    gpu.create_texture(&wgpu::TextureDescriptor {
        label: Some("encode threads measurement target"),
        size: wgpu::Extent3d {
            width: shape.width,
            height: shape.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// One cold frame of one page shape on `threads` threads: build a device, build the
/// scene on it, render once, and report what the encode cost.
fn sample(
    adapter: Option<String>,
    shape: &Shape,
    threads: usize,
) -> (Duration, Duration, Counters) {
    let mut device = Device::headless(&Options {
        adapter,
        instrument_encode: true,
        encode_threads: threads,
        ..Options::default()
    })
    .expect("an adapter");
    device.wait_until_warm();
    let scene = build(&mut device, shape);
    let viewport = Viewport::full(shape.width, shape.height, Affine::IDENTITY);
    let texture = target_texture(&device, shape);
    let frame = device
        .render(&scene, &viewport, Target::Texture(&texture))
        .expect("the page draws");
    let timings = frame.timings();
    let geometry = timings
        .phases
        .iter()
        .find(|(name, _)| *name == "encode: geometry")
        .map_or(Duration::ZERO, |(_, d)| *d);
    (timings.encode, geometry, frame.counters())
}

/// The counters a run compares across thread counts: every one of them is an exact
/// function of the scene, so a difference is a defect and not a measurement.
fn signature(counters: Counters) -> [u32; 7] {
    [
        counters.commands,
        counters.tiles,
        counters.distinct_outlines,
        counters.atlas_distinct_keys,
        counters.segments,
        counters.clip_residue_regions,
        counters.clip_residue_tiles,
    ]
}

fn load_average() -> String {
    std::fs::read_to_string("/proc/loadavg").map_or_else(
        |_| "unknown".into(),
        |line| {
            line.split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ")
        },
    )
}

fn main() {
    let mut args = std::env::args().skip(1);
    let adapter = args.next().filter(|a| a != "-");
    let rounds: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(3);
    let counts: Vec<usize> = args.next().map_or_else(
        || vec![1, 2, 4, 8, 24],
        |list| list.split(',').filter_map(|n| n.parse().ok()).collect(),
    );

    println!("load average before: {}", load_average());
    for shape in SHAPES {
        let mut encode: Vec<Vec<Duration>> = counts.iter().map(|_| Vec::new()).collect();
        let mut geometry: Vec<Vec<Duration>> = counts.iter().map(|_| Vec::new()).collect();
        let mut seen: Option<[u32; 7]> = None;
        for round in 0..rounds {
            // Rotate the order each round, so drift falls on every configuration rather
            // than on whichever ran last (`doc/HANDOVER.md`'s wall-clock trap).
            for offset in 0..counts.len() {
                let slot = (offset + round) % counts.len();
                let started = Instant::now();
                let (enc, geo, counters) = sample(adapter.clone(), shape, counts[slot]);
                let whole = started.elapsed();
                encode[slot].push(enc);
                geometry[slot].push(geo);
                let signed = signature(counters);
                match seen {
                    None => seen = Some(signed),
                    Some(first) => assert_eq!(
                        first, signed,
                        "{}: the counters moved with the thread count",
                        shape.name
                    ),
                }
                let _ = whole;
            }
        }
        let first = seen.expect("a round ran");
        println!(
            "\n{} — {} commands, {} tiles, {} distinct outlines, {} atlas keys, \
             {} segments, {} residue regions, {} residue tiles",
            shape.name, first[0], first[1], first[2], first[3], first[4], first[5], first[6],
        );
        for (slot, threads) in counts.iter().enumerate() {
            let min = |v: &[Duration]| v.iter().copied().min().unwrap_or_default();
            println!(
                "  {threads:>2} thread(s): encode min {:?}, geometry min {:?}",
                min(&encode[slot]),
                min(&geometry[slot]),
            );
        }
    }
    println!("\nload average after: {}", load_average());
}
