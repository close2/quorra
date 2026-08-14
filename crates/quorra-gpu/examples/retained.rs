//! What an unchanged frame costs when it does not encode itself again (ADR 0048).
//!
//! ADR 0045 priced this on a throwaway `git worktree` in which `Device::render` held the
//! previous frame's `Encoded` — **0.154 ms against 1.538** on the dense-text archetype.
//! This is the same measurement through the API that was built instead, and it is a
//! better instrument in one respect that matters: **both variants are in one binary and
//! run on one device**, so the round-robin is a round-robin of two calls rather than of
//! two builds, and there is no second target directory, no second compilation and no
//! drift between them to argue about.
//!
//! ```text
//! cargo run --release -p quorra-gpu --example retained [-- <adapter substring> [rounds]]
//! ```
//!
//! # How it measures, and why each of those decisions
//!
//! - **Round-robin, one frame of each variant per round.** Contiguous per-variant blocks
//!   put a factor of three between two runs of `examples/rect_lane.rs` at load 85; drift
//!   has to fall on both variants or it is not measurement.
//! - **Minima, never means.** `doc/HANDOVER.md`'s first trap: this machine is somebody's
//!   desktop, and the medians here carry outliers of several milliseconds on *both*
//!   variants. The load average is printed beside the numbers so a reader can discount
//!   the run rather than the conclusion.
//! - **Into a retained `Target::Texture`**, created once. A `Readback` frame would put a
//!   copy-out and a map — the largest single cost a frame has — on top of the thing being
//!   measured, and a `Surface` frame would block on vsync.
//! - **The counters are checked against `tests/archetypes.rs`'s recorded row** before any
//!   number is printed. That is what says this binary encoded §6.2's page and not a
//!   lookalike, and it is the same discipline the callgrind harness of ADR 0045 used.
//! - **The pixels of the two variants are compared**, once, through a `Readback` pair, so
//!   the run reports byte identity rather than assuming what `tests/retained_frame.rs`
//!   asserts.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use std::time::{Duration, Instant};

use quorra_gpu::{Counters, Device, EncodeSource, Options, RetainedScene, Target, Viewport, wgpu};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, OutlineId, Paint, Point, Scene, SceneBuilder,
    Segment,
};

/// The brief's window scale (§6.2): the size both baselines were measured at.
const WIDTH: u32 = 1191;
const HEIGHT: u32 = 1684;

/// §6.2's page: dense text at the corpus's 99th percentile and its measured reuse —
/// 4 320 placements over 818 outlines, 40 of them under a curve clip.
const COMMANDS: u32 = 4_320;
const DISTINCT: u32 = 818;
const SEGMENTS: u32 = 12;
const SIDE: f32 = 11.0;
const CLIPS: u32 = 2;
const CLIPPED: u32 = 40;

/// `tests/archetypes.rs`'s recorded row for dense text: commands, culled, distinct
/// outlines, atlas keys, clip regions, tiles, layer textures. The gate on this
/// example's scene being the archetype's scene.
const BASELINE: [u32; 7] = [4320, 0, 818, 2164, 1, 40, 0];

fn signature(counters: &Counters) -> [u32; 7] {
    [
        counters.commands,
        counters.commands_culled,
        counters.distinct_outlines,
        counters.atlas_distinct_keys,
        counters.clip_distinct_regions,
        counters.tiles,
        counters.layer_textures,
    ]
}

/// A closed curve of `segments` cubics about the origin, `side` across — the shape a
/// letterform has for costing purposes, as in `tests/archetypes.rs`.
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

/// Where the `index`th command lands: a reading-order grid over the page.
fn position(index: u32, side: f32) -> Affine {
    let step = side + 3.5;
    let columns = ((WIDTH as f32 - 16.0) / step).max(1.0) as u32;
    let x = 8.0 + (index % columns) as f32 * step + side * 0.5;
    let y = 12.0 + (index / columns) as f32 * (side + 4.25) + side * 0.5;
    Affine::translate(x, y % (HEIGHT as f32 - 24.0))
}

/// The dense-text archetype, built on this device.
fn build(device: &mut Device) -> Scene {
    let outlines: Vec<OutlineId> = (0..DISTINCT)
        .map(|i| {
            let side = SIDE * (1.0 + (i % 5) as f32 * 0.05);
            device.upload_outline(&outline_of(SEGMENTS, side)).unwrap()
        })
        .collect();
    let mut builder = SceneBuilder::new();
    let clips: Vec<quorra_scene::ClipId> = (0..CLIPS)
        .map(|i| {
            builder
                .clip(
                    outlines[(i as usize) % outlines.len()],
                    position(i, SIDE * 6.0),
                    FillRule::NonZero,
                    None,
                )
                .unwrap()
        })
        .collect();
    let ink = Color::new(0.12, 0.13, 0.16, 1.0);
    for index in 0..COMMANDS {
        let clip = (index < CLIPPED).then(|| clips[(index as usize) % clips.len()]);
        builder
            .fill(
                outlines[(index as usize) % outlines.len()],
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
    builder.finish()
}

/// One frame's host and device spans.
#[derive(Clone, Copy)]
struct Record {
    wall: Duration,
    encode: Duration,
    upload: Duration,
    execute: Duration,
}

/// Minimum, second, third and median of a column, in milliseconds — the four numbers
/// `doc/PLAN.md`'s round-robin tables are read from.
fn column(records: &[Record], of: impl Fn(&Record) -> Duration) -> String {
    let mut values: Vec<f64> = records.iter().map(|r| of(r).as_secs_f64() * 1e3).collect();
    values.sort_by(f64::total_cmp);
    let at = |i: usize| values.get(i).copied().unwrap_or(f64::NAN);
    format!(
        "min {:.3}  2nd {:.3}  3rd {:.3}  median {:.3}",
        at(0),
        at(1),
        at(2),
        at(values.len() / 2)
    )
}

fn load_average() -> String {
    std::fs::read_to_string("/proc/loadavg").map_or_else(
        |_| "unknown".into(),
        |s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "),
    )
}

fn target_texture(device: &Device) -> wgpu::Texture {
    let (gpu, _) = device.wgpu();
    gpu.create_texture(&wgpu::TextureDescriptor {
        label: Some("retained measurement target"),
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

/// The two variants draw the same bytes, checked rather than assumed.
fn compare_pixels(device: &mut Device, scene: &Scene, retained: &mut RetainedScene) {
    let viewport = Viewport::full(WIDTH, HEIGHT, Affine::IDENTITY);
    let encoded = device
        .render(scene, &viewport, Target::Readback)
        .unwrap()
        .into_raster()
        .unwrap();
    let replayed = device
        .render_retained(retained, &viewport, Target::Readback)
        .unwrap()
        .into_raster()
        .unwrap();
    let differing = encoded
        .pixels()
        .iter()
        .zip(replayed.pixels())
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "pixels: {differing} of {} bytes differ between an encoded frame and a replayed one",
        encoded.pixels().len()
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let adapter = args.next();
    let rounds: usize = args.next().and_then(|r| r.parse().ok()).unwrap_or(40);

    let mut device = Device::headless(&Options {
        adapter: adapter.clone(),
        ..Options::default()
    })
    .expect("an adapter");
    device.wait_until_warm();
    println!("adapter: {}", device.description());
    println!("scene:   dense text, {COMMANDS} commands over {DISTINCT} outlines, {WIDTH}x{HEIGHT}");

    let scene = build(&mut device);
    let mut retained = RetainedScene::new(scene.clone());
    let texture = target_texture(&device);
    let viewport = Viewport::full(WIDTH, HEIGHT, Affine::IDENTITY);

    // Two warm-up frames each: the first fills the atlas and compiles what a first frame
    // compiles, and neither variant is being measured on that.
    for _ in 0..2 {
        device
            .render(&scene, &viewport, Target::Texture(&texture))
            .unwrap();
        device
            .render_retained(&mut retained, &viewport, Target::Texture(&texture))
            .unwrap();
    }

    let mut encoded_runs: Vec<Record> = Vec::with_capacity(rounds);
    let mut replayed_runs: Vec<Record> = Vec::with_capacity(rounds);
    let mut counters: Option<Counters> = None;

    for _ in 0..rounds {
        let started = Instant::now();
        let frame = device
            .render(&scene, &viewport, Target::Texture(&texture))
            .unwrap();
        let wall = started.elapsed();
        assert_eq!(frame.encode_source(), EncodeSource::Encoded);
        counters.get_or_insert_with(|| frame.counters());
        encoded_runs.push(Record {
            wall,
            encode: frame.timings().encode,
            upload: frame.timings().upload,
            execute: frame.timings().execute,
        });

        let started = Instant::now();
        let frame = device
            .render_retained(&mut retained, &viewport, Target::Texture(&texture))
            .unwrap();
        let wall = started.elapsed();
        assert_eq!(
            frame.encode_source(),
            EncodeSource::Replayed,
            "the retained frame re-encoded: something invalidated it, and the numbers \
             below would be two encodes rather than a comparison"
        );
        // The signature rather than the whole row: `bytes_uploaded` is about *this*
        // frame's transfers, and after the warm-up neither variant uploads a glyph tile,
        // so the two agree here — but that is a property of a warm atlas, not of a
        // replay, and asserting it would be asserting the wrong thing.
        assert_eq!(
            signature(&frame.counters()),
            signature(&counters.unwrap()),
            "a replayed frame counts what the encode counted"
        );
        replayed_runs.push(Record {
            wall,
            encode: frame.timings().encode,
            upload: frame.timings().upload,
            execute: frame.timings().execute,
        });
    }

    let counters = counters.unwrap();
    assert_eq!(
        signature(&counters),
        BASELINE,
        "this is not the dense-text archetype: (commands, culled, outlines, atlas keys, \
         clip regions, tiles, layer textures)"
    );

    compare_pixels(&mut device, &scene, &mut retained);
    println!(
        "retained: {} bytes held by the handle",
        retained.retained_bytes()
    );
    println!("load average: {}", load_average());
    println!("rounds: {rounds}, round-robin, one frame of each per round\n");

    for (name, runs) in [
        ("re-encoded", &encoded_runs),
        ("replayed  ", &replayed_runs),
    ] {
        println!("{name} wall    {}", column(runs, |r| r.wall));
        println!("{name} encode  {}", column(runs, |r| r.encode));
        println!("{name} upload  {}", column(runs, |r| r.upload));
        println!("{name} execute {}", column(runs, |r| r.execute));
    }
}
