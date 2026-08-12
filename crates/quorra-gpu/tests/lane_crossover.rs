//! The instrument behind ADR 0028's lane criterion: what each coverage lane costs on
//! the same page, with and without an atlas that will hold the page's tiles.
//!
//! **A measurement, not a gate.** It is `#[ignore]`d and asserts nothing: a wall clock
//! on a shared machine is evidence, not a build failure (the same reasoning as
//! `archetypes.rs`'s absurdly-long test). Run it when the lane criterion is in question:
//!
//! ```text
//! cargo test --release -p quorra-gpu --test lane_crossover -- --ignored --nocapture
//! ```
//!
//! ADR 0027 stated a crossover as a tile area and asked its successor to re-derive the
//! table rather than inherit the constant. This is that table, and re-deriving it is
//! what found that tile area was the wrong axis altogether: the CPU lane's advantage is
//! the *atlas*, so the two columns to compare are "the atlas holds this tile" against
//! "it does not", and at every size in both columns the answer is the same one.
//!
//! Three things the shape of the harness is deliberate about:
//!
//! - **A texture target, not a readback.** Copying a 52 MB page out and demultiplying it
//!   is 15-20 ms paid identically by both lanes, and it hides what is being compared.
//! - **Distinct outlines as well as one shared.** `doc/corpus-profile.md` measured 1.33
//!   placements per distinct outline at the median of the caller's 995 pages, so a page
//!   whose glyphs are all new is the *normal* case and not a pathology — and it is the
//!   case where the atlas cannot help however large it is.
//! - **Cold and warm both reported.** Cold is a page opened; warm is the fastest of
//!   eight re-renders, which is a page scrolled back to. The atlas moves them apart by
//!   an order of magnitude, which is the whole finding.

// Test-file lint policy as in `m1.rs`: a fixture that cannot run must fail loudly, and
// the arithmetic here is a fixture's own — page sizes and grid indices, all constants of
// this file.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use std::time::Instant;

use quorra_gpu::{Coverage, Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, OutlineId, Paint, Point, Scene, SceneBuilder,
    Segment,
};

/// The page every fixture is drawn on: a letter page at roughly 3×.
const PAGE: u32 = 3600;

/// The tile sizes the table walks, as the width of one shape in device pixels.
const SIDES: [u32; 10] = [50, 100, 150, 200, 300, 400, 500, 700, 900, 1200];

/// A glyph-shaped outline: a twelve-pointed star of 24 straight segments in a box of
/// `side × 1.3 side`.
///
/// Straight segments, and enough of them that the triangle count is a glyph's rather
/// than a blob's — ADR 0026's floor is priced in triangles per placement, so a fixture
/// with four of them would measure a lane no page takes. `jitter` moves one point, which
/// is enough to make the outline, and so the atlas key, distinct from every other one.
fn star(device: &mut Device, side: u32, jitter: f32) -> OutlineId {
    let (w, h) = (side as f32, side as f32 * 1.3);
    let (cx, cy) = (w * 0.5, h * 0.5);
    let mut segments = Vec::new();
    for point in 0..24_u32 {
        let angle = point as f32 * std::f32::consts::TAU / 24.0;
        let radius: f32 = if point % 2 == 0 { 0.48 } else { 0.22 };
        let x = radius.mul_add(w * angle.cos(), cx) + if point == 3 { jitter } else { 0.0 };
        let y = radius.mul_add(h * angle.sin(), cy);
        segments.push(if point == 0 {
            Segment::MoveTo(Point::new(x, y))
        } else {
            Segment::LineTo(Point::new(x, y))
        });
    }
    segments.push(Segment::Close);
    device.upload_outline(&segments).unwrap()
}

/// How the page's placements share outlines, which is what the census counts.
///
/// The third case is the one a page of text at a middling zoom actually is: a few
/// hundred letterforms, each drawn a handful of times, together outgrowing the atlas.
/// It is where the cache is worth having *and* cannot hold everything, which no other
/// row of these tables reaches.
#[derive(Debug, Clone, Copy)]
enum Sharing {
    /// Every placement its own outline: the corpus median, where the atlas is admitted
    /// to and buys nothing.
    Distinct,
    /// One outline per `n` placements.
    Repeated(u32),
    /// One outline for the whole page: dense text's best case.
    Shared,
}

impl Sharing {
    fn label(self) -> String {
        match self {
            Self::Distinct => "distinct".into(),
            Self::Repeated(n) => format!("{n} uses each"),
            Self::Shared => "one shared".into(),
        }
    }
}

/// A grid of stars filling the page, sharing outlines as `sharing` says.
fn grid(device: &mut Device, side: u32, sharing: Sharing) -> (Scene, u32) {
    let (w, h) = (side as f32, side as f32 * 1.3);
    let columns = (PAGE / side).max(1);
    let rows = (PAGE / (h as u32).max(1)).max(1);
    let shared = star(device, side, 0.0);
    let mut uploaded: Vec<OutlineId> = Vec::new();
    let mut builder = SceneBuilder::new();
    for row in 0..rows {
        for column in 0..columns {
            let index = row * columns + column;
            let outline = match sharing {
                Sharing::Distinct => star(device, side, index as f32 * 1e-3),
                Sharing::Repeated(uses) => {
                    let slot = (index / uses.max(1)) as usize;
                    if slot >= uploaded.len() {
                        uploaded.push(star(device, side, slot as f32 * 1e-3));
                    }
                    uploaded[slot]
                }
                Sharing::Shared => shared,
            };
            builder
                .fill(
                    outline,
                    Affine::translate(column as f32 * w, row as f32 * h),
                    FillRule::NonZero,
                    Paint::Solid(Color::new(0.1, 0.1, 0.1, 1.0)),
                    None,
                    BlendMode::Normal,
                    Compose::SrcOver,
                    None,
                )
                .unwrap();
        }
    }
    (builder.finish(), columns * rows)
}

/// One lane's answer on one page: the first frame, the second, the fastest of eight
/// after those, and what the frame said about itself.
///
/// The second frame is its own column because ADR 0029's cross-frame memory pays there:
/// a page of single-use shapes draws on the device the first time it is seen and enters
/// the atlas the second, so a design that only reported cold and warm would hide the
/// one slow frame between them.
struct Run {
    cold: f64,
    second: f64,
    warm: f64,
    shapes: u32,
    tiles: u32,
}

fn measure(coverage: Coverage, side: u32, sharing: Sharing, atlas: u64) -> Run {
    let mut device = Device::headless(&Options {
        coverage,
        atlas_budget: atlas,
        // Budgets out of the way: this harness is about time, and a refusal is the
        // business of the tests that are about bytes.
        max_frame_bytes: 1024 * 1024 * 1024,
        max_resource_bytes: 2 * 1024 * 1024 * 1024,
        ..Options::default()
    })
    .expect("some adapter must exist");
    device.wait_until_warm();
    let (scene, shapes) = grid(&mut device, side, sharing);
    let viewport = Viewport::full(PAGE, PAGE, Affine::IDENTITY);
    let texture = device.wgpu().0.create_texture(&wgpu::TextureDescriptor {
        label: Some("lane crossover target"),
        size: wgpu::Extent3d {
            width: PAGE,
            height: PAGE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let draw = |device: &mut Device| {
        let start = Instant::now();
        let frame = device
            .render(&scene, &viewport, Target::Texture(&texture))
            .expect("the fixture is inside every budget");
        (
            start.elapsed().as_secs_f64() * 1000.0,
            frame.timings().encode.as_secs_f64() * 1000.0,
            frame.counters().tiles,
        )
    };
    let (cold, _, tiles) = draw(&mut device);
    let (second, _, _) = draw(&mut device);
    let mut warm = f64::MAX;
    for _ in 0..8 {
        let (elapsed, _, _) = draw(&mut device);
        warm = warm.min(elapsed);
    }
    Run {
        cold,
        second,
        warm,
        shapes,
        tiles,
    }
}

fn table(sharing: Sharing, atlas: u64) {
    println!(
        "\n--- {} outlines, atlas {} KiB, page {PAGE}x{PAGE}, 16 samples, ms ---",
        sharing.label(),
        atlas / 1024
    );
    println!(
        "{:>5} {:>9} {:>6} | {:>7} {:>7} {:>7} | {:>7} {:>7} {:>7} | {:>6}",
        "side",
        "texels",
        "shapes",
        "cpu 1st",
        "cpu 2nd",
        "cpu warm",
        "gpu 1st",
        "gpu 2nd",
        "gpu warm",
        "tiles"
    );
    for side in SIDES {
        let texels = side * (side as f32 * 1.3) as u32;
        let cpu = measure(Coverage::Cpu, side, sharing, atlas);
        let gpu = measure(Coverage::Gpu, side, sharing, atlas);
        println!(
            "{side:>5} {texels:>9} {:>6} | {:>7.1} {:>7.1} {:>7.1} | {:>7.1} {:>7.1} {:>7.1} | \
             {:>6}",
            cpu.shapes, cpu.cold, cpu.second, cpu.warm, gpu.cold, gpu.second, gpu.warm, gpu.tiles
        );
    }
}

/// The four tables ADR 0028's criterion was read off.
#[test]
#[ignore = "a wall clock is a measurement here, not a gate; see the module comment"]
fn lane_crossover_table() {
    println!("adapters {:?}", Device::adapter_names());
    // The default atlas, where a tile under an eighth of 8 MiB is cached and the rest
    // are not — both regimes the criterion distinguishes, on one budget.
    table(Sharing::Distinct, quorra_gpu::startup::DEFAULT_ATLAS_BUDGET);
    table(Sharing::Shared, quorra_gpu::startup::DEFAULT_ATLAS_BUDGET);
    // And an atlas too small to hold any of them, which is what a page of very large
    // shapes looks like to any budget: the same comparison with the cache taken away.
    table(Sharing::Distinct, 64 * 1024);
    table(Sharing::Shared, 64 * 1024);
    // The case in between, and the only one where the atlas both earns its place and
    // runs out of room: every outline drawn three times, into a cache a quarter the
    // size of what the page asks of it.
    table(Sharing::Repeated(3), 1024 * 1024);
}
