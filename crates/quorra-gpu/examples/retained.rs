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
use quorra_pages::{Archetype, DENSE_TEXT, GLYPH_PAGE, GlyphPage, Recorded};
use quorra_scene::{Affine, OutlineId, Scene};

/// The brief's window scale (§6.2): the size both baselines were measured at.
const WIDTH: u32 = 1191;
const HEIGHT: u32 = 1684;

/// §6.2's page: `quorra_pages::DENSE_TEXT`, the corpus's 99th percentile at its measured
/// reuse — 4 320 placements over 818 outlines, 40 of them under a curve clip.
///
/// **This file carried a private copy of that page until 2026-08-17, and the copy is
/// exactly what ADR 0060 exists for.** ADR 0057 changed what a clipped mark costs; the
/// row copied in here still said 40 tiles for a page that had stopped drawing any, and
/// this example **panicked at its own signature gate on `main` for two days** — nothing
/// caught it, because `cargo test` neither builds nor runs an example.
const SHAPE: &Archetype = &DENSE_TEXT;

/// A frame's counters as the row `quorra-pages` records, field by named field.
///
/// The recorded row is no longer a copy: it is the page's own, the one
/// `tests/archetypes.rs` compares against. Only the mapping is written twice, and a
/// mapping that is wrong fails immediately rather than rotting.
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

/// The dense-text archetype, built on this device.
fn build(device: &mut Device) -> Scene {
    let outlines: Vec<OutlineId> = quorra_pages::outlines(SHAPE)
        .iter()
        .map(|path| device.upload_outline(path).expect("an archetype outline"))
        .collect();
    quorra_pages::scene(SHAPE, &outlines, None).expect("the dense-text archetype builds")
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

/// The atlas budget the section below runs against, and the page it runs on.
///
/// **The band ADR 0050 is about is narrow, and that is the point.** A working set far
/// larger than the atlas never repacked even before it — ADR 0024's byte test blocks
/// that — and one that fits the atlas never overflows at all. What used to repack for
/// ever is the page in between: one that fits the atlas **by bytes** and does not fit it
/// **by shelves**, because a shelf packer wastes the remainder of two divisions and the
/// byte test cannot see either.
///
/// A quarter-megabyte atlas (512×512) holding a page of 107 letterforms magnified
/// fourfold is measured to be exactly that: 107 distinct keys, 102 of them resident,
/// five tiles a frame onto the scratch sheet. The archetype above is *not* usable here —
/// at 4× it culls 94 % of its commands and the 234 keys that survive ask for 736 KB,
/// which is the other regime — so this section carries its own page, and asserts the
/// condition rather than trusting these numbers to keep meaning what they mean.
const OVERFLOW_ATLAS: u64 = 256 * 1024;

/// The overflow section's page: **`quorra_pages::GLYPH_PAGE`** — 5 933 fills over 107
/// letterforms of five widths and seven heights, on a 14.5 × 15.25 grid, the same
/// definition `examples/zoom.rs` and `examples/floor.rs` draw.
///
/// Not the archetype above, and not a variation on it. The band this section measures is
/// a relation between the tile sizes a page produces and the two divisions a shelf
/// packer performs, so the page is the one the band was *measured* on: substituting a
/// different letterform changes the tile size, which moves the working set, which is how
/// an instrument silently stops measuring what it names. The assertion at the end is the
/// guard, and it is what caught two earlier substitutions of exactly this kind.
///
/// This file's copy of it said "verbatim" and was not: it drew in the *archetypes'* ink
/// rather than the glyph page's. Nothing here reads a colour, which is why nobody
/// noticed and why reconciling it moves no number (ADR 0060 §5).
const OVERFLOW_PAGE: &GlyphPage = &GLYPH_PAGE;

/// The overflow page, built on this device.
fn overflow_page(device: &mut Device) -> Scene {
    let outlines: Vec<OutlineId> = quorra_pages::glyph_outlines(OVERFLOW_PAGE)
        .iter()
        .map(|path| device.upload_outline(path).expect("a letterform"))
        .collect();
    quorra_pages::glyph_scene(OVERFLOW_PAGE, &outlines).expect("the glyph page builds")
}

/// **Does the atlas settle?** — the second measurement in this file, and a property
/// rather than a clock (ADR 0050).
///
/// A page whose glyph tiles overflow the atlas used to invalidate its own retained
/// encode: the tile that fell through to the scratch sheet made the device repack, the
/// repack moved every texel origin the encode had just been stored under, and the next
/// frame did all of it again. The instrument for that is not a duration — it is the
/// **sequence of encode sources**, which is the same on an idle machine and on this one.
///
/// `E` is a frame that encoded, `.` a frame that replayed. A settled atlas reads
/// `E...........`; the pathology reads `EEEEEEEEEEEE`.
fn overflow_section(adapter: Option<&str>, frames: usize) {
    let mut device = Device::headless(&Options {
        adapter: adapter.map(str::to_owned),
        atlas_budget: OVERFLOW_ATLAS,
        ..Options::default()
    })
    .expect("an adapter");
    device.wait_until_warm();
    let scene = overflow_page(&mut device);
    // `quorra_pages::zoomed` — `examples/zoom.rs` asks for the same frame through the
    // same function, so the two examples magnify one page about one point.
    let viewport = Viewport::full(WIDTH, HEIGHT, quorra_pages::zoomed(OVERFLOW_PAGE, 4.0));
    let mut retained = RetainedScene::new(scene);

    let mut sources = String::new();
    let mut repacks = 0_u32;
    let mut last: Option<Counters> = None;
    for _ in 0..frames {
        let frame = device
            .render_retained(&mut retained, &viewport, Target::Readback)
            .expect("a page too large for the atlas still draws — through the scratch sheet");
        sources.push(if frame.encode_source() == EncodeSource::Encoded {
            'E'
        } else {
            '.'
        });
        repacks += u32::from(frame.counters().atlas_repacked);
        last = Some(frame.counters());
    }
    let counters = last.expect("at least one frame");
    println!(
        "\na page the atlas cannot hold — {} letterforms at 4×, atlas {OVERFLOW_ATLAS} bytes",
        OVERFLOW_PAGE.distinct
    );
    println!(
        "  working set {} bytes over {} distinct keys; {} resident, {} tiles on the scratch sheet",
        counters.atlas_working_set_bytes,
        counters.atlas_distinct_keys,
        counters.atlas_entries,
        counters.tiles
    );
    println!("  encode sources [{sources}], repacks {repacks}");
    assert!(
        counters.tiles > 0 && counters.atlas_working_set_bytes <= OVERFLOW_ATLAS,
        "this section measures nothing unless the atlas refused a tile for a working set \
         that fits it by bytes: {counters:?}"
    );
}

/// What one round of the round-robin needs. A struct rather than six arguments, because
/// five of them are borrows of the same device and the order they are passed in is not
/// something a reader should have to remember.
struct RoundRobin<'a> {
    device: &'a mut Device,
    scene: &'a Scene,
    retained: &'a mut RetainedScene,
    texture: &'a wgpu::Texture,
    viewport: &'a Viewport<'a>,
    rounds: usize,
}

/// One frame of each variant per round, and the two assertions that say the comparison
/// is a comparison: the encoded frame encoded, and the replayed one replayed and counted
/// what the encode counted.
fn round_robin(run: &mut RoundRobin<'_>) -> (Vec<Record>, Vec<Record>, Counters) {
    let mut encoded_runs: Vec<Record> = Vec::with_capacity(run.rounds);
    let mut replayed_runs: Vec<Record> = Vec::with_capacity(run.rounds);
    let mut counters: Option<Counters> = None;
    let record = |wall, frame: &quorra_gpu::Frame| Record {
        wall,
        encode: frame.timings().encode,
        upload: frame.timings().upload,
        execute: frame.timings().execute,
    };
    for _ in 0..run.rounds {
        let started = Instant::now();
        let frame = run
            .device
            .render(run.scene, run.viewport, Target::Texture(run.texture))
            .expect("the dense-text archetype draws");
        let wall = started.elapsed();
        assert_eq!(frame.encode_source(), EncodeSource::Encoded);
        counters.get_or_insert_with(|| frame.counters());
        encoded_runs.push(record(wall, &frame));

        let started = Instant::now();
        let frame = run
            .device
            .render_retained(run.retained, run.viewport, Target::Texture(run.texture))
            .expect("a retained frame draws");
        let wall = started.elapsed();
        assert_eq!(
            frame.encode_source(),
            EncodeSource::Replayed,
            "the retained frame re-encoded: something invalidated it, and the numbers \
             reported would be two encodes rather than a comparison"
        );
        // The recorded row rather than the whole of `Counters`: `bytes_uploaded` is about
        // *this* frame's transfers, and after the warm-up neither variant uploads a glyph
        // tile, so the two agree here — but that is a property of a warm atlas, not of a
        // replay, and asserting it would be asserting the wrong thing.
        assert_eq!(
            recorded(&frame.counters()),
            recorded(&counters.expect("the encoded frame ran first")),
            "a replayed frame counts what the encode counted"
        );
        replayed_runs.push(record(wall, &frame));
    }
    (
        encoded_runs,
        replayed_runs,
        counters.expect("at least one round"),
    )
}

/// `--check`: the smallest run that executes every assertion this example makes.
///
/// One round of the round-robin and one frame of the overflow section, which reaches
/// every `assert!` in the file. `cargo test` neither builds nor runs an example, so an
/// assertion here is a comment until something runs it — and this is the file that
/// proved it, by panicking at its own signature gate on `main` for two days (ADR 0060).
/// CI runs `--check` for every example named in `.github/workflows/ci.yml`.
fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let check = args.iter().any(|arg| arg == "--check");
    args.retain(|arg| arg != "--check");
    let mut args = args.into_iter();
    let adapter = args.next();
    let rounds: usize = if check {
        1
    } else {
        args.next().and_then(|r| r.parse().ok()).unwrap_or(40)
    };

    let mut device = Device::headless(&Options {
        adapter: adapter.clone(),
        ..Options::default()
    })
    .expect("an adapter");
    device.wait_until_warm();
    println!("adapter: {}", device.description());
    println!(
        "scene:   {}, {} commands over {} outlines, {WIDTH}x{HEIGHT}",
        SHAPE.name, SHAPE.commands, SHAPE.distinct
    );

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

    let (encoded_runs, replayed_runs, counters) = round_robin(&mut RoundRobin {
        device: &mut device,
        scene: &scene,
        retained: &mut retained,
        texture: &texture,
        viewport: &viewport,
        rounds,
    });

    // The signature gate — the one that rotted. The row is no longer a copy: it is the
    // page's own, and a change to what the page costs now moves the row this compares
    // against and the row `tests/archetypes.rs` compares against in the same edit.
    assert_eq!(
        recorded(&counters),
        SHAPE.recorded.expect("dense text is a priced page"),
        "this is not `quorra_pages::DENSE_TEXT` as that crate records it"
    );

    compare_pixels(&mut device, &scene, &mut retained);
    println!(
        "retained: {} bytes held by the handle",
        retained.retained_bytes()
    );

    if check {
        // One frame is enough to reach the overflow section's assertion; twelve is what
        // makes its `E...........` readable, and a one-round table is not a measurement.
        overflow_section(adapter.as_deref(), 1);
        println!(
            "check: the dense-text archetype's row holds, a replay counts what the \
                  encode counted, and the atlas refused a tile for a working set that fits"
        );
        return;
    }

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

    overflow_section(adapter.as_deref(), 12);
}
