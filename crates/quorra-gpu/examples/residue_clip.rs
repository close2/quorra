//! What a page of curve-clipped marks costs to encode (ADR 0049).
//!
//! The artwork archetype — 900 commands, 185 curve clips, 600 commands under them — is
//! the corpus's p99 clip shape and the row `doc/PLAN.md` carries for this seam. Its
//! encode is where the residue clip lives, so this measures the encode and its geometry
//! split rather than a frame: the device is about 4 % of a frame of this page, and a
//! readback would put the largest single cost a frame has on top of the thing being
//! measured.
//!
//! ```text
//! cargo run --release -p quorra-gpu --example residue_clip [-- <adapter substring> [rounds]]
//! ```
//!
//! # How it measures, and why
//!
//! - **Headless, into a `Target::Texture` created once**, so no surface, no vsync and no
//!   copy-out is in the span.
//! - **Minima, never means** (`doc/HANDOVER.md`'s first trap): this machine is somebody's
//!   desktop. The load average is printed beside the numbers so a reader can discount the
//!   run rather than the conclusion.
//! - **The counters are printed with the clocks**, and `clip_residue_regions` +
//!   `clip_residue_tiles` are exact functions of the scene: they say what the encode did
//!   without believing a clock at all, which is what makes this comparable across two
//!   builds on a loaded machine.
//! - **The first frame is reported separately.** It carries the atlas fill and any
//!   first-use pipeline compile, and averaging it into the steady state hides both.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use std::time::{Duration, Instant};

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, GroupSpec, LineCap, LineJoin, OutlineId, Paint,
    Point, Scene, SceneBuilder, Segment, Stroke,
};

/// The brief's window scale (§6.2), which is what the archetype's counts were taken at.
const WIDTH: u32 = 1191;
const HEIGHT: u32 = 1684;

/// `tests/archetypes.rs`'s artwork row, and the fields this binary needs of it. The
/// geometry that realises it is shared with the gate by construction rather than by
/// import — an example cannot reach a test's module — so the counters below are what
/// says this encoded that page.
const COMMANDS: u32 = 900;
const DISTINCT: u32 = 300;
const SEGMENTS: u32 = 24;
const SIDE: f32 = 60.0;
const STROKES: u32 = 405;
const CLIPS: u32 = 185;
const CLIPPED: u32 = 600;
const GROUPS: u32 = 8;
const BLENDED_GROUPS: u32 = 4;

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

fn position(index: u32, side: f32) -> Affine {
    let step = side + 3.5;
    let columns = ((WIDTH as f32 - 16.0) / step).max(1.0) as u32;
    let x = 8.0 + (index % columns) as f32 * step + side * 0.5;
    let y = 12.0 + (index / columns) as f32 * (side + 4.25) + side * 0.5;
    Affine::translate(x, y % (HEIGHT as f32 - 24.0))
}

fn emit(
    builder: &mut SceneBuilder,
    outlines: &[OutlineId],
    clips: &[quorra_scene::ClipId],
    index: u32,
) {
    let outline = outlines[(index as usize) % outlines.len()];
    let clip =
        (index < CLIPPED && !clips.is_empty()).then(|| clips[(index as usize) % clips.len()]);
    let ink = Color::new(0.12, 0.13, 0.16, 1.0);
    if index < STROKES {
        builder
            .stroke(
                outline,
                position(index, SIDE),
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
                position(index, SIDE),
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

fn build(device: &mut Device) -> Scene {
    let outlines: Vec<OutlineId> = (0..DISTINCT.max(1))
        .map(|i| {
            let side = SIDE * (1.0 + (i % 5) as f32 * 0.05);
            device.upload_outline(&outline_of(SEGMENTS, side)).unwrap()
        })
        .collect();
    let mut builder = SceneBuilder::new();
    let clips: Vec<quorra_scene::ClipId> = (0..CLIPS)
        .map(|i| {
            let outline = outlines[(i as usize) % outlines.len()];
            builder
                .clip(outline, position(i, SIDE * 6.0), FillRule::NonZero, None)
                .unwrap()
        })
        .collect();
    let per_group = (COMMANDS / 4)
        .checked_div(GROUPS)
        .map_or(0, |per| per.max(1));
    let grouped = per_group * GROUPS;
    for group in 0..GROUPS {
        let spec = GroupSpec {
            alpha: 0.8,
            blend: if group < BLENDED_GROUPS {
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
                    emit(body, &outlines, &clips, group * per_group + step);
                }
                Ok(())
            })
            .unwrap();
    }
    for index in grouped..COMMANDS {
        emit(&mut builder, &outlines, &clips, index);
    }
    builder.finish()
}

/// The measurement target, created once: a frame that allocated one would be measuring
/// the allocator.
fn target_texture(device: &Device) -> wgpu::Texture {
    let (gpu, _) = device.wgpu();
    gpu.create_texture(&wgpu::TextureDescriptor {
        label: Some("residue clip measurement target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
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
    let adapter = args.next();
    let rounds: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(20);

    let mut device = Device::headless(&Options {
        adapter,
        instrument_encode: true,
        ..Options::default()
    })
    .expect("an adapter");
    device.wait_until_warm();
    let scene = build(&mut device);
    let viewport = Viewport::full(WIDTH, HEIGHT, Affine::IDENTITY);
    let texture = target_texture(&device);

    let mut encode = Vec::with_capacity(rounds);
    let mut geometry = Vec::with_capacity(rounds);
    let mut wall = Vec::with_capacity(rounds);
    let mut first = None;
    for round in 0..=rounds {
        let started = Instant::now();
        let frame = device
            .render(&scene, &viewport, Target::Texture(&texture))
            .expect("the artwork page draws");
        let elapsed = started.elapsed();
        let timings = frame.timings();
        let phase = |name: &str| {
            timings
                .phases
                .iter()
                .find(|(n, _)| *n == name)
                .map_or(Duration::ZERO, |(_, d)| *d)
        };
        if round == 0 {
            let counters = frame.counters();
            first = Some((elapsed, timings.encode, phase("encode: geometry")));
            println!(
                "counters: {} commands, {} tiles, {} residue regions, {} residue tiles, \
                 {} layer textures",
                counters.commands,
                counters.tiles,
                counters.clip_residue_regions,
                counters.clip_residue_tiles,
                counters.layer_textures,
            );
        } else {
            wall.push(elapsed);
            encode.push(timings.encode);
            geometry.push(phase("encode: geometry"));
        }
    }

    let min = |v: &[Duration]| v.iter().copied().min().unwrap_or_default();
    let median = |v: &mut Vec<Duration>| {
        v.sort_unstable();
        v[v.len() / 2]
    };
    let (fw, fe, fg) = first.unwrap();
    println!("load average: {}", load_average());
    println!("first frame:  wall {fw:?}, encode {fe:?}, geometry {fg:?}");
    println!(
        "steady min:   wall {:?}, encode {:?}, geometry {:?}",
        min(&wall),
        min(&encode),
        min(&geometry)
    );
    println!(
        "steady median: wall {:?}, encode {:?}, geometry {:?}",
        median(&mut wall),
        median(&mut encode),
        median(&mut geometry)
    );
}
