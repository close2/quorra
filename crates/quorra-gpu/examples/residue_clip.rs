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

use quorra_gpu::{Counters, Device, Options, Target, Viewport};
use quorra_pages::{ARTWORK, Archetype, Recorded};
use quorra_scene::{Affine, Scene};

/// The brief's window scale (§6.2), which is what the archetype's counts were taken at.
const WIDTH: u32 = 1191;
const HEIGHT: u32 = 1684;

/// The page: `quorra_pages::ARTWORK`, the same definition `tests/archetypes.rs` gates.
///
/// **It used to be a private copy of the generator in this file**, and re-cutting the
/// page meant editing it here as well as in the test and in three other examples
/// (ADR 0060). The counters printed below are no longer what says this encoded that
/// page — that is true by construction now — but they are still printed, because they
/// are exact functions of the scene and are what makes two runs on a loaded machine
/// comparable at all.
const SHAPE: &Archetype = &ARTWORK;

/// Build the page on this device.
fn build(device: &mut Device) -> Scene {
    let outlines: Vec<quorra_scene::OutlineId> = quorra_pages::outlines(SHAPE)
        .iter()
        .map(|path| device.upload_outline(path).expect("an archetype outline"))
        .collect();
    quorra_pages::scene(SHAPE, &outlines, None).expect("the artwork archetype builds")
}

/// The frame's counters as the row `quorra-pages` records, field by named field.
fn recorded(counters: &Counters) -> Recorded {
    Recorded {
        commands: u64::from(counters.commands),
        commands_culled: u64::from(counters.commands_culled),
        distinct_outlines: u64::from(counters.distinct_outlines),
        atlas_distinct_keys: u64::from(counters.atlas_distinct_keys),
        clip_distinct_regions: u64::from(counters.clip_distinct_regions),
        tiles: u64::from(counters.tiles),
        layer_textures: u64::from(counters.layer_textures),
        clip_residue_regions: u64::from(counters.clip_residue_regions),
        clip_residue_tiles: u64::from(counters.clip_residue_tiles),
        coverage_texels: counters.coverage.texels,
    }
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

/// `--check`: the smallest run that executes every assertion this example makes.
///
/// `cargo test` neither builds nor runs an example, so an assertion here is a comment
/// until something runs it (ADR 0060). CI runs `--check` for every example named in
/// `.github/workflows/ci.yml`, and `tests/example_checks.rs` fails if an example exists
/// that the workflow does not name. It prints one `check:` line and no statistics: a
/// one-round measurement is not a measurement, and must not read like one.
fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let check = args.iter().any(|arg| arg == "--check");
    args.retain(|arg| arg != "--check");
    let mut args = args.into_iter();
    let adapter = args.next();
    let rounds: usize = if check {
        1
    } else {
        args.next().and_then(|n| n.parse().ok()).unwrap_or(20)
    };

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
    let mut staging = Vec::with_capacity(rounds);
    let mut recording = Vec::with_capacity(rounds);
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
            // The signature gate. It cannot rot into the defect ADR 0060 exists for —
            // the row it compares against is `quorra-pages`', the same one
            // `tests/archetypes.rs` compares against — but it still catches the frame
            // that drew something else than the encode this file's numbers are
            // attributed to.
            assert_eq!(
                recorded(&counters),
                SHAPE.recorded.expect("artwork is a priced page"),
                "this frame is not the artwork archetype as `quorra-pages` records it",
            );
            println!("coverage: {} texels on the sheet", counters.coverage.texels);
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
            staging.push(phase("encode: staging"));
            recording.push(phase("encode: recording"));
        }
    }

    if check {
        println!("check: the artwork archetype drew and its counters are the recorded row");
        return;
    }

    let (fw, fe, fg) = first.expect("round 0 ran");
    println!("load average: {}", load_average());
    println!("first frame:  wall {fw:?}, encode {fe:?}, geometry {fg:?}");
    report(&mut Steady {
        wall,
        encode,
        geometry,
        staging,
        recording,
    });
}

/// The steady-state rounds, one column per phase.
struct Steady {
    wall: Vec<Duration>,
    encode: Vec<Duration>,
    geometry: Vec<Duration>,
    staging: Vec<Duration>,
    recording: Vec<Duration>,
}

/// Minima, the fastest frame's phase shares, and medians.
fn report(steady: &mut Steady) {
    let min = |v: &[Duration]| v.iter().copied().min().unwrap_or_default();
    println!(
        "steady min:   wall {:?}, encode {:?}, geometry {:?}, staging {:?}, recording {:?}",
        min(&steady.wall),
        min(&steady.encode),
        min(&steady.geometry),
        min(&steady.staging),
        min(&steady.recording)
    );
    // The three phases are the point of this instrument since ADR 0023's 2026-08-17
    // amendment, so it prints all three rather than leaving `recording` to be inferred
    // from a subtraction the reader has to do — and prints the *same* frame's three, at
    // the frame whose encode was the fastest, so the shares below sum to that encode.
    let fastest = steady
        .encode
        .iter()
        .enumerate()
        .min_by_key(|(_, d)| **d)
        .map_or(0, |(i, _)| i);
    let share = |part: Duration| {
        let whole = steady.encode[fastest].as_secs_f64();
        if whole > 0.0 {
            100.0 * part.as_secs_f64() / whole
        } else {
            0.0
        }
    };
    println!(
        "fastest encode {:?}: geometry {:.1} %, staging {:.1} %, recording {:.1} %",
        steady.encode[fastest],
        share(steady.geometry[fastest]),
        share(steady.staging[fastest]),
        share(steady.recording[fastest]),
    );
    let median = |v: &mut Vec<Duration>| {
        v.sort_unstable();
        v[v.len() / 2]
    };
    println!(
        "steady median: wall {:?}, encode {:?}, geometry {:?}, staging {:?}, recording {:?}",
        median(&mut steady.wall),
        median(&mut steady.encode),
        median(&mut steady.geometry),
        median(&mut steady.staging),
        median(&mut steady.recording)
    );
}
