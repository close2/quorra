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
use quorra_pages::{ARTWORK, Archetype, CALLERS_DRAWING, DENSE_TEXT_UNCLIPPED, MEDIAN_PAGE};
use quorra_scene::{Affine, OutlineId, Scene};

/// The four page shapes this sweep runs, all four defined in `quorra-pages` (ADR 0060).
///
/// Each was a private copy of the archetype generator in this file until 2026-08-17, and
/// **two of the four were not the archetype their comment named**. Naming them properly
/// is what a register of pages buys: `DENSE_TEXT_UNCLIPPED` is dense text without its
/// two curve clips, which is a different page and always was, and `CALLERS_DRAWING` is
/// the caller's file at its own 58 009 commands rather than the 1 200-command `DRAWING`
/// archetype. **Neither is changed here** — ADR 0054's sweep was measured on these pages,
/// and re-cutting one in the round that moved it is the trap
/// `doc/notes-clipped-instrument.md` §3.4 names.
///
/// # Why the dense-text row stays unclipped — decided 2026-08-23, not deferred again
///
/// `doc/HANDOVER.md` carried this as an open question from 2026-08-17: whether the sweep
/// should run `DENSE_TEXT` — the archetype, with its two curve clips — and put its 40
/// residue-clipped marks into the "does not divide" column where `ARTWORK` already is.
/// **Measured, and declined.** Three numbers, none of them a clock:
///
/// - **Those 40 marks really do not divide**, and that is a fact about the dispatch rather
///   than about a timing. `Encoder::deferrable_bounds` returns `None` whenever the chain
///   has a residue, and the glyph lane's guard requires `residues.is_none()`, so a
///   residue-clipped mark reaches neither queued lane and is flattened and scanned on the
///   walk. The premise is sound; it is the *size* of it that decides this.
/// - **It is 1.84 % of the page's coverage work.** The archetype's serial residue is
///   40 tiles of **8 956** coverage texels (`quorra_pages::DENSE_TEXT`'s recorded row);
///   the parallel side is the atlas working set, **476 892** bytes — identical on both
///   pages, measured here on llvmpipe, since the clips change no glyph tile. 8 956 of
///   485 848. Carried through Amdahl at the ~2.8× this page actually reaches at 24
///   threads, that moves the scaling ratio by about **3 %**.
/// - **`ARTWORK` says the same thing 396× louder.** Its serial residue is 3 542 360
///   coverage texels against dense text's 8 956. The archetype would not be a second
///   entry in the "does not divide" column; it would be the unclipped page plus a quarter
///   of one percent of artwork's.
///
/// Against that 3 %, this instrument's reproducibility on this machine: two sweeps of the
/// *same* configuration on 2026-08-23, 9 rounds at load average 19 and 15 rounds at load
/// average 101, reported 1-thread `encode: geometry` minima of **6.17 ms and 11.68 ms**
/// for the archetype — an 89 % spread, and in the lower run the clipped page came out
/// *faster* than the unclipped one, which the dispatch above says is impossible. The
/// effect the switch exists to expose is thirty times smaller than the noise the
/// instrument had that day. A shape that earns its place in a sweep is one the sweep can
/// resolve; this one is not, and adding it would cost ADR 0054's series its comparability
/// for a row nobody could read.
///
/// What the question was really about — that "dense text" here and "dense text" in
/// `tests/archetypes.rs` were two pages under one name — is closed already, and by naming
/// rather than by measuring: the row this prints says `dense text, unclipped`.
///
/// The order is the order the sweep prints: the caller's page first, because it is the
/// one their `doc/QUORRA_ENCODE_THREADS.md` asks about; artwork second, because its 600
/// residue-clipped marks are the case that does **not** divide; then the atlas shape and
/// the floor.
const SHAPES: [&Archetype; 4] = [
    &CALLERS_DRAWING,
    &ARTWORK,
    &DENSE_TEXT_UNCLIPPED,
    &MEDIAN_PAGE,
];

/// Build a shape's scene on this device.
///
/// A different side per outline is what makes them distinct shapes rather than one shape
/// uploaded many times: the caller's page has 58 003 of those.
fn build(device: &mut Device, shape: &Archetype) -> Scene {
    let outlines: Vec<OutlineId> = quorra_pages::outlines(shape)
        .iter()
        .map(|path| device.upload_outline(path).expect("an outline"))
        .collect();
    quorra_pages::scene(shape, &outlines, None).expect("a page builds")
}

fn target_texture(device: &Device, shape: &Archetype) -> wgpu::Texture {
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
    shape: &Archetype,
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

/// `--check`: the smallest run that executes every assertion this example makes.
///
/// One round over two thread counts on every shape, which is what reaches the "the
/// counters moved with the thread count" assertion — it needs two configurations of one
/// page and nothing more. `cargo test` neither builds nor runs an example, so an
/// assertion here is a comment until something runs it (ADR 0060); CI runs `--check`
/// for every example named in `.github/workflows/ci.yml`.
///
/// It prints no minima. A single round of a wall clock on this machine is not a
/// measurement, and a line that looks like one is worse than no line.
fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let check = args.iter().any(|arg| arg == "--check");
    args.retain(|arg| arg != "--check");
    let mut args = args.into_iter();
    let adapter = args.next().filter(|a| a != "-");
    let rounds: usize = if check {
        1
    } else {
        args.next().and_then(|n| n.parse().ok()).unwrap_or(3)
    };
    let counts: Vec<usize> = if check {
        vec![1, 2]
    } else {
        args.next().map_or_else(
            || vec![1, 2, 4, 8, 24],
            |list| list.split(',').filter_map(|n| n.parse().ok()).collect(),
        )
    };

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
        if check {
            continue;
        }
        for (slot, threads) in counts.iter().enumerate() {
            let min = |v: &[Duration]| v.iter().copied().min().unwrap_or_default();
            println!(
                "  {threads:>2} thread(s): encode min {:?}, geometry min {:?}",
                min(&encode[slot]),
                min(&geometry[slot]),
            );
        }
    }
    if check {
        println!("\ncheck: every shape's counters are the same at 1 thread and at 2");
        return;
    }
    println!("\nload average after: {}", load_average());
}
