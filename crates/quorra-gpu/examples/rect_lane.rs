//! The caller's §19 measurement: what does a `Rect` command save over a `Fill` of the
//! same four-edge outline?
//!
//! Their `doc/QUORRA_FEEDBACK.md` §19 reports that not one page of their 995-page corpus
//! emits a `Command::Rect`: every rectangle a document draws arrives as a `Fill` whose
//! outline happens to be one, because `pdf_render::Command::Fill` carries an outline and
//! their translation hands it over without asking whether it is four axis-aligned edges.
//! Whether recognising the shape is worth writing depends on one number that only this
//! side can produce — *what the `Rect` lane saves inside `encode`* — at the two sizes
//! their corpus profile says a page actually contains: **median 12 commands, p99 4 320**.
//!
//! Three scenes draw the identical set of device rectangles, and the example checks that
//! they are identical rather than assuming it:
//!
//! - `rect` — [`SceneBuilder::rect`], the analytic lane of ADR 0007
//!   (`encode.rs::encode_rect`): a clip intersection and one instance, no rasterisation;
//! - `fill (shared)` — one uploaded unit-square outline placed by a scale-and-translate,
//!   the friendliest case for the fill lanes, because one shape at many placements is
//!   what the glyph atlas and the census exist for;
//! - `fill (distinct)` — one uploaded outline per rectangle, which is what a document's
//!   varied rules, underlines and table cells actually are, and what the census calls
//!   placed-once.
//!
//! **The recogniser lives on the fill path already, but only on the shaded arm**
//! (`encode.rs:1473`, `StoredOutline::rect_hint`): a *solid* fill of a rectangular
//! outline takes the GPU triangle lane, the glyph lane or the scratch coverage path like
//! any other shape. So the difference these three columns show is a real difference, not
//! two names for one lane.
//!
//! The three are timed **round-robin**, one frame of each per round, and reported as
//! minima: `encode` is a host span, this machine is somebody's desktop, and ADR 0040 is
//! the price of believing a wall clock that was not measured that way.
//!
//! Run: `cargo run --release -p quorra-gpu --example rect_lane`
//! Both adapters. Check `uptime` first and quote it beside the numbers.

// Scene coordinates are bounded by the page size, so the casts are exact there.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]

use std::time::Duration;

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, Paint, Point, Rect, Scene, SceneBuilder, Segment,
    Size,
};

/// The caller's window, and the size every number in `doc/PLAN.md` is quoted at.
const WIDTH: u32 = 1191;
const HEIGHT: u32 = 1684;

/// The corpus profile's two rows: the median page and the p99 dense one.
const SIZES: [u32; 2] = [12, 4_320];

const INK: Color = Color::new(0.1, 0.1, 0.1, 1.0);

/// Where the `i`th rectangle sits and how big it is — one grid, shared by all three
/// scene builders so that the marks are the same marks.
///
/// The sizes vary (35 distinct pairs) because a page's rectangles do: a rule, an
/// underline and a table cell are not one shape at three places.
fn placement(i: u32) -> Rect {
    let left = f64::from(i % 80).mul_add(14.5, 3.25) as f32;
    let top = f64::from(i / 80).mul_add(15.25, 4.5) as f32;
    let width = (i % 7) as f32 * 0.5 + 9.75;
    let height = (i % 5) as f32 * 0.75 + 11.5;
    Rect::new(
        Point::new(left, top),
        Point::new(left + width, top + height),
    )
}

/// The four axis-aligned edges of `size` at the origin, closed — §8.5.2.1's `re`
/// written out, which is exactly what the caller's translation hands over today.
fn rect_outline(size: Size) -> [Segment; 5] {
    let (width, height) = (size.width, size.height);
    [
        Segment::MoveTo(Point::new(0.0, 0.0)),
        Segment::LineTo(Point::new(width, 0.0)),
        Segment::LineTo(Point::new(width, height)),
        Segment::LineTo(Point::new(0.0, height)),
        Segment::Close,
    ]
}

fn rect_scene(n: u32) -> Scene {
    let mut builder = SceneBuilder::new();
    for i in 0..n {
        builder
            .rect(placement(i), Affine::IDENTITY, INK, None, None)
            .unwrap();
    }
    builder.finish()
}

/// One outline, many placements: the unit square scaled and translated onto the same
/// grid. Axis-preserving, so it reaches the same device rectangle to the bit.
fn fill_scene_shared(device: &mut Device, n: u32) -> Scene {
    let unit = device
        .upload_outline(&rect_outline(Size {
            width: 1.0,
            height: 1.0,
        }))
        .unwrap();
    let mut builder = SceneBuilder::new();
    for i in 0..n {
        let at = placement(i);
        let size = at.size();
        builder
            .fill(
                unit,
                Affine {
                    a: size.width,
                    b: 0.0,
                    c: 0.0,
                    d: size.height,
                    e: at.min.x,
                    f: at.min.y,
                },
                FillRule::NonZero,
                Paint::Solid(INK),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
    }
    builder.finish()
}

/// One outline per rectangle, geometry baked and placed by a translation: the shape a
/// page of varied rules and cells has, and the one the census sees only once.
fn fill_scene_distinct(device: &mut Device, n: u32) -> Scene {
    let mut builder = SceneBuilder::new();
    for i in 0..n {
        let at = placement(i);
        let outline = device.upload_outline(&rect_outline(at.size())).unwrap();
        builder
            .fill(
                outline,
                Affine::translate(at.min.x, at.min.y),
                FillRule::NonZero,
                Paint::Solid(INK),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
    }
    builder.finish()
}

/// What one scene's timed run produced: the fastest `encode` of the rounds, and the
/// counts that say which lane it took.
struct Lane {
    encode: Duration,
    tiles: u32,
    atlas_keys: u32,
}

/// Time every scene once per round rather than each scene to exhaustion, so a load
/// spike falls on all of them instead of on whichever was running.
///
/// This machine is somebody's desktop and `doc/HANDOVER.md`'s first trap is that wall
/// clocks lie under load; ADR 0040 is the whole cost of not doing this. Contiguous
/// blocks put a factor of three between two runs of this example at load 85.
fn round_robin(
    device: &mut Device,
    scenes: &[(&str, Scene)],
    viewport: &Viewport<'_>,
    target: &wgpu::Texture,
    rounds: u32,
) -> Vec<Lane> {
    let mut lanes: Vec<Lane> = scenes
        .iter()
        .map(|_| Lane {
            encode: Duration::MAX,
            tiles: 0,
            atlas_keys: 0,
        })
        .collect();
    // Round zero is the cold one — where each lane rasterises whatever it rasterises —
    // and it is thrown away: the steady frame is what a page turn actually pays.
    for round in 0..=rounds {
        for (lane, (_, scene)) in lanes.iter_mut().zip(scenes) {
            let frame = device
                .render(scene, viewport, Target::Texture(target))
                .unwrap();
            if round > 0 {
                lane.encode = lane.encode.min(frame.timings().encode);
            }
            lane.tiles = frame.counters().tiles;
            lane.atlas_keys = frame.counters().atlas_distinct_keys;
        }
    }
    lanes
}

/// How many of the two rasters' bytes differ, and by how much at worst. §19's second
/// condition is that the lanes draw *exactly* the same mark; this is what checks it.
fn difference(a: &[u8], b: &[u8]) -> (usize, u8) {
    a.iter()
        .zip(b)
        .filter(|(left, right)| left != right)
        .fold((0usize, 0u8), |(count, worst), (left, right)| {
            (count.saturating_add(1), worst.max(left.abs_diff(*right)))
        })
}

fn readback(device: &mut Device, scene: &Scene) -> Vec<u8> {
    let viewport = Viewport::full(WIDTH, HEIGHT, Affine::IDENTITY);
    device
        .render(scene, &viewport, Target::Readback)
        .unwrap()
        .into_raster()
        .unwrap()
        .into_pixels()
}

fn milliseconds(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// The three scenes at one command count, timed round-robin and checked against each
/// other.
fn measure_size(device: &mut Device, target: &wgpu::Texture, n: u32, rounds: u32) {
    let viewport = Viewport::full(WIDTH, HEIGHT, Affine::IDENTITY);
    let scenes = [
        ("rect", rect_scene(n)),
        ("fill (shared)", fill_scene_shared(device, n)),
        ("fill (distinct)", fill_scene_distinct(device, n)),
    ];
    let lanes = round_robin(device, &scenes, &viewport, target, rounds);
    let baseline = milliseconds(lanes[0].encode);
    let reference = readback(device, &scenes[0].1);
    for (lane, (label, scene)) in lanes.iter().zip(&scenes) {
        let (differing, worst) = difference(&reference, &readback(device, scene));
        println!(
            "{n:>5} cmds  {label:<16} encode {:>8.4} ms  ({:>6.2}× rect)  \
             tiles {:>5}  atlas keys {:>5}  \
             bytes differing from rect: {differing:>8}, worst {worst:>3}",
            milliseconds(lane.encode),
            milliseconds(lane.encode) / baseline,
            lane.tiles,
            lane.atlas_keys,
        );
    }
}

fn main() {
    for adapter in ["llvmpipe", "RADV"] {
        let Ok(mut device) = Device::headless(&Options {
            adapter: Some(adapter.into()),
            ..Options::default()
        }) else {
            eprintln!("adapter '{adapter}' unavailable; skipped");
            continue;
        };
        device.wait_until_warm();
        println!("\n== {} ==", device.description());
        let target = {
            let (gpu, _) = device.wgpu();
            gpu.create_texture(&wgpu::TextureDescriptor {
                label: Some("rect_lane target"),
                size: wgpu::Extent3d {
                    width: WIDTH,
                    height: HEIGHT,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
        };
        for n in SIZES {
            // Enough rounds for the minimum to settle, and fewer of the dense one
            // because each of its frames also draws 4 320 marks.
            let rounds = if n > 1_000 { 60 } else { 400 };
            measure_size(&mut device, &target, n, rounds);
        }
    }
}
