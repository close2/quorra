//! Six page shapes, generated here, priced by counters that cannot drift.
//!
//! # Why this file exists
//!
//! Until it, every performance fixture in this tree was invented: `perf_gate.rs` draws
//! 5 933 **rectangles** because the brief's dense page has 5 933 glyphs. Measured over
//! the caller's 995-page corpus, **not one page emits a single `Command::Rect`** — the
//! command that fixture uses is one no document sends, and since ADR 0047 a document's
//! rectangles reach the same lane as fills instead — and glyph reuse is **1.33
//! placements per distinct outline at the median**, not the 55 that fixture assumes. A
//! gate built on those assumptions cannot see a regression on the shapes documents
//! actually have, which is how a twentyfold zoom cliff and a sixteenfold sheet lived
//! here undetected until the caller's own gate or an example found them.
//!
//! `doc/corpus-profile.md` holds the measurement, its date and how to redo it. **The
//! numbers below are all that came back from it** — no document, no display list, no
//! reference to that project: delete it from the machine and this file still compiles,
//! runs and means the same thing. What is checked in is the same kind of thing already
//! checked into half our ADRs, which is a number somebody measured.
//!
//! # What is gated, and why counters rather than clocks
//!
//! Every `Counters` field is an exact function of the scene and the viewport, so a
//! baseline of them compares by **equality on any machine** — no thresholds, no
//! flakiness under load, and a regression names itself. `tiles` would have caught the
//! atlas cliff, `bytes_uploaded` the sheet committed at the device's maximum width, and
//! `layer_textures` the frame that priced a pair per plan. The clocks are printed and
//! gated far more loosely, because a wall clock on a loaded runner is context rather
//! than evidence (CLAUDE.md principle 2).
//!
//! Every archetype renders on a **fresh device**, because every frame the caller's gate
//! measures is a cold-atlas frame — the warm second render is the exception on a page
//! turn, not the rule, and measuring it is how a fixture flatters itself.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use quorra_gpu::{Counters, Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, GroupSpec, ImageFilter, ImageSpec, LineCap,
    LineJoin, OutlineId, Paint, Point, Scene, SceneBuilder, Segment, Stroke,
};

/// One page shape, as a set of measured counts. Every field is a number from
/// `doc/corpus-profile.md`; the geometry that realises them is this file's own.
struct Archetype {
    name: &'static str,
    /// Drawing commands, groups excluded.
    commands: u32,
    /// Distinct outlines uploaded — `commands / distinct` is the reuse the atlas sees.
    distinct: u32,
    /// Segments per outline, as a closed curve.
    segments: u32,
    /// Device side of one shape, in pixels at 1×.
    side: f32,
    /// How many of the commands are strokes rather than fills.
    strokes: u32,
    /// Image placements, and the side of the (synthetic) image behind them.
    images: u32,
    image_side: u32,
    /// Clip regions defined, and how many commands draw under one.
    clips: u32,
    clipped: u32,
    /// Whether those clips are axis-aligned rectangles, which ADR 0007 resolves to a
    /// rectangle at encode time, or curves, which leave a residue every clipped command
    /// must multiply into a coverage tile. The two are different lanes and the corpus
    /// has both.
    rect_clips: bool,
    /// Groups wrapping runs of the commands, and how many of those carry a blend mode.
    groups: u32,
    blended_groups: u32,
}

/// The median corpus page: twelve commands, nine outlines. Most of a corpus is this,
/// and what it measures is the per-frame floor rather than any lane.
const MEDIAN_PAGE: Archetype = Archetype {
    rect_clips: false,
    name: "median page",
    commands: 12,
    distinct: 9,
    segments: 8,
    side: 11.0,
    strokes: 0,
    images: 0,
    image_side: 0,
    clips: 0,
    clipped: 0,
    groups: 0,
    blended_groups: 0,
};

/// A dense page of text at the corpus's 99th percentile — and at its *measured* reuse,
/// which is five placements per outline rather than the fifty-five a fixture built from
/// the brief's one page assumes.
const DENSE_TEXT: Archetype = Archetype {
    rect_clips: false,
    name: "dense text",
    commands: 4_320,
    distinct: 818,
    segments: 12,
    side: 11.0,
    strokes: 0,
    images: 0,
    image_side: 0,
    clips: 2,
    clipped: 40,
    groups: 0,
    blended_groups: 0,
};

/// Artwork: strokes beside fills, clips on most of it, a few blended groups. The shape
/// of the Illustrator and `InDesign` pages that carry every group feature we have.
const ARTWORK: Archetype = Archetype {
    rect_clips: false,
    name: "artwork",
    commands: 900,
    distinct: 300,
    segments: 24,
    side: 60.0,
    strokes: 405,
    images: 0,
    image_side: 0,
    clips: 185,
    clipped: 600,
    groups: 8,
    blended_groups: 4,
};

/// A page of photographs: the corpus's 99th percentile for image placements, over text.
const IMAGE_PAGE: Archetype = Archetype {
    rect_clips: true,
    name: "image page",
    commands: 200,
    distinct: 60,
    segments: 8,
    side: 11.0,
    strokes: 0,
    images: 32,
    image_side: 128,
    clips: 4,
    clipped: 32,
    groups: 0,
    blended_groups: 0,
};

/// The corpus's clip mountain, at a fifth of its size: the page that prompted it defines
/// **15 004** clip regions, and 3 000 costs the same lanes in a suite that has to finish.
/// Nothing here was invented — a page like it exists, and it is why
/// `clip_distinct_regions` is a counter rather than a hope.
const CLIP_MOUNTAIN: Archetype = Archetype {
    rect_clips: true,
    name: "clip mountain",
    commands: 1_200,
    distinct: 200,
    segments: 8,
    side: 24.0,
    strokes: 0,
    images: 0,
    image_side: 0,
    clips: 1_200,
    clipped: 1_200,
    groups: 0,
    blended_groups: 0,
};

/// The corpus's largest page, scaled down: that page holds **66 309** commands over
/// 65 978 distinct outlines. What distinguishes it is not its size but its **reuse of
/// exactly one** — every command carries its own outline, so the atlas never answers and
/// every command rasterises — and 1 500 commands hold that property while leaving a debug
/// build able to finish (an unoptimised rasteriser is twenty times slower, and this file
/// runs on every `cargo test`). The absolute number is in `doc/corpus-profile.md`.
const GIANT: Archetype = Archetype {
    rect_clips: true,
    name: "giant",
    commands: 1_500,
    distinct: 1_500,
    segments: 8,
    side: 9.0,
    strokes: 0,
    images: 0,
    image_side: 0,
    clips: 0,
    clipped: 0,
    groups: 0,
    blended_groups: 0,
};

/// A drawing: tens of thousands of small filled polygons, each its own outline, each
/// carrying fifty-odd path segments, and no text, no image, no group and no clip
/// anywhere on the page.
///
/// **The caller's own file, scaled down.** That page is 49.7 MB and one content stream:
/// **58 009 commands — 58 003 fills, six strokes — over 3 011 879 path segments, 51.9 a
/// fill**, and at its fit view a mark is about three device pixels across
/// (`pdf-viewer/doc/QUORRA_ENCODE_THREADS.md` §1). It is a geological cross-section
/// exported by Inkscape, and it is every drawing, map, plan and chart in a corpus.
///
/// What distinguishes it from [`GIANT`] — which also reuses exactly one outline — is the
/// **ratio**: fifty-two segments flattened for a nine-pixel tile, where giant flattens
/// eight for eighty. Encode on this shape is flattening and scanline work almost to the
/// exclusion of everything else, which is the property no other archetype here had, and
/// the reason a lane measured on the rest of this list would have been measured on the
/// wrong page. 1 200 commands hold the ratio while leaving a debug build able to finish,
/// which is the same bargain [`GIANT`] states.
const DRAWING: Archetype = Archetype {
    rect_clips: false,
    name: "drawing",
    commands: 1_200,
    distinct: 1_200,
    segments: 52,
    side: 3.0,
    strokes: 6,
    images: 0,
    image_side: 0,
    clips: 0,
    clipped: 0,
    groups: 0,
    blended_groups: 0,
};

const ARCHETYPES: [&Archetype; 7] = [
    &MEDIAN_PAGE,
    &DENSE_TEXT,
    &ARTWORK,
    &IMAGE_PAGE,
    &CLIP_MOUNTAIN,
    &GIANT,
    &DRAWING,
];

const WIDTH: u32 = 1191;
const HEIGHT: u32 = 1684;

/// A closed curve of `segments` segments about the origin, `side` across: the shape a
/// letterform has for costing purposes — curved, closed, and filled under the non-zero
/// rule.
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

/// An axis-aligned rectangle as a path. `quorra_scene`'s own detector recognises this
/// shape, so `ADR 0007` resolves a clip built from it to a rectangle — no residue, and
/// no coverage tile for the commands under it.
fn rect_path(half: f32) -> Vec<Segment> {
    vec![
        Segment::MoveTo(Point::new(-half, -half)),
        Segment::LineTo(Point::new(half, -half)),
        Segment::LineTo(Point::new(half, half)),
        Segment::LineTo(Point::new(-half, half)),
        Segment::Close,
    ]
}

/// Where the `index`th command lands: a reading-order grid over the page.
fn position(index: u32, side: f32) -> Affine {
    let step = side + 3.5;
    let columns = ((WIDTH as f32 - 16.0) / step).max(1.0) as u32;
    let x = 8.0 + (index % columns) as f32 * step + side * 0.5;
    let y = 12.0 + (index / columns) as f32 * (side + 4.25) + side * 0.5;
    Affine::translate(x, y % (HEIGHT as f32 - 24.0))
}

/// The archetype's resources: outlines to place, and one image if it has any.
fn upload(
    device: &mut Device,
    shape: &Archetype,
) -> (Vec<OutlineId>, Option<quorra_scene::ImageId>) {
    let outlines = (0..shape.distinct.max(1))
        .map(|i| {
            let side = shape.side * (1.0 + (i % 5) as f32 * 0.05);
            device
                .upload_outline(&outline_of(shape.segments, side))
                .unwrap()
        })
        .collect();
    let image = (shape.images > 0).then(|| {
        let side = shape.image_side.max(1);
        let pixels: Vec<u8> = (0..side * side * 4).map(|i| (i % 251) as u8).collect();
        device
            .upload_image(&ImageSpec {
                width: side,
                height: side,
                data: Arc::from(pixels.into_boxed_slice()),
            })
            .unwrap()
    });
    (outlines, image)
}

/// The archetype's clip chains.
///
/// A clip is either an axis-aligned rectangle, which ADR 0007 resolves at encode time,
/// or a curve, which leaves a residue that every clipped command multiplies into a
/// coverage tile. Both are ordinary on real pages and they are different lanes with
/// different costs, which is why `rect_clips` is a field rather than a constant.
///
/// A rectangular clip here covers the page and differs from its neighbours by a hair, so
/// it admits every command under it: the subject is the resolver and
/// `clip_distinct_regions`, not culling, which has a gate of its own.
fn define_clips(
    builder: &mut SceneBuilder,
    device: &mut Device,
    shape: &Archetype,
    outlines: &[OutlineId],
) -> Vec<quorra_scene::ClipId> {
    let rectangle = shape
        .rect_clips
        .then(|| device.upload_outline(&rect_path(1.0)).unwrap());
    let centre = Affine::translate(WIDTH as f32 * 0.5, HEIGHT as f32 * 0.5);
    (0..shape.clips)
        .map(|i| {
            let outline = rectangle.unwrap_or_else(|| outlines[(i as usize) % outlines.len()]);
            let transform = if shape.rect_clips {
                let half = HEIGHT as f32 * 0.6 + i as f32 * 0.01;
                Affine::scale(half, half).then(centre)
            } else {
                position(i, shape.side * 6.0)
            };
            builder
                .clip(outline, transform, FillRule::NonZero, None)
                .unwrap()
        })
        .collect()
}

/// One drawing command: a stroke while the archetype's stroke budget lasts, a fill
/// after it, under the clip its index selects.
fn emit(
    builder: &mut SceneBuilder,
    shape: &Archetype,
    outlines: &[OutlineId],
    clips: &[quorra_scene::ClipId],
    index: u32,
) {
    let outline = outlines[(index as usize) % outlines.len()];
    let clip =
        (index < shape.clipped && !clips.is_empty()).then(|| clips[(index as usize) % clips.len()]);
    let ink = Color::new(0.12, 0.13, 0.16, 1.0);
    if index < shape.strokes {
        builder
            .stroke(
                outline,
                position(index, shape.side),
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
                position(index, shape.side),
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

/// Build the archetype's scene on this device. Deterministic: the same archetype
/// produces the same scene, command for command, on every run and every machine.
fn build(device: &mut Device, shape: &Archetype) -> Scene {
    let (outlines, image) = upload(device, shape);
    let mut builder = SceneBuilder::new();
    let clips = define_clips(&mut builder, device, shape, &outlines);

    // Groups take the first commands, in equal runs, so nesting is real rather than a
    // wrapper around nothing — and `grouped` is derived from `per_group`, so the totals
    // are exact rather than rounded.
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
    for index in 0..shape.images {
        if let Some(image) = image {
            let side = shape.image_side as f32;
            builder
                .image(
                    image,
                    Affine::scale(side, side).then(position(index, side)),
                    1.0,
                    ImageFilter::Nearest,
                    None,
                    BlendMode::Normal,
                    None,
                )
                .unwrap();
        }
    }
    builder.finish()
}

/// A device with nothing in its caches, which is what every frame of a page turn is.
fn cold_device() -> Device {
    let device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs");
    device.wait_until_warm();
    device
}

fn render(device: &mut Device, scene: &Scene) -> (Counters, Duration) {
    let started = Instant::now();
    let frame = device
        .render(
            scene,
            &Viewport::full(WIDTH, HEIGHT, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("an archetype must draw");
    let counters = frame.counters();
    let raster = frame.into_raster().expect("Readback carries its pixels");
    assert_eq!(raster.pixels().len(), (WIDTH * HEIGHT * 4) as usize);
    (counters, started.elapsed())
}

/// What each archetype costs, recorded from a run and explained line by line — a
/// baseline nobody can account for is a baseline nobody can defend.
///
/// `(commands, culled, distinct outlines, atlas keys, clip regions, tiles, layer
/// textures, residue regions, residue tiles)`. Every field is an exact function of the
/// scene and the viewport, so these compare by equality on any machine and any adapter.
/// Recorded 2026-08-12; the last two joined it on 2026-08-15 (ADR 0049).
///
/// - **median page** — twelve fills over nine outlines at twelve distinct sub-pixel
///   phases, so twelve keys; all cached, so no tile touches the sheet.
/// - **dense text** — 4 320 placements over 818 outlines collapse to 2 164 keys, which
///   is the quantised phase doing its job. The **40 tiles** are the 40 commands under a
///   *curve* clip: each multiplies the clip's residue into a coverage tile of its own.
///   The two clips resolve to one rectangle, the shape living in the residue. Those two
///   chains are **2 residue regions** and no residue tile: each is rasterised once and
///   the 40 commands take windows on it (ADR 0049).
/// - **artwork** — 684 top-level nodes are 676 draws plus 8 groups (`Counters::commands`
///   counts the scene's top level). **600 tiles** are the 600 curve-clipped commands,
///   and **3 layer textures** are the root's accumulator, one group's at a time, and the
///   copy of the pixels that group's composite covers — ADR 0020's depth pricing showing
///   its work on eight sibling groups, at ADR 0038's one texture per plan. It was 4 while
///   a plan ping-ponged between two. **185 residue regions against 600 tiles** is
///   ADR 0049's whole subject: 185 chains, 600 commands under them, and one
///   rasterisation each rather than one per command. A residue-tile count above zero
///   here would mean the admission rule had changed its mind about this page.
/// - **image page** — 200 fills and 32 images under *rectangular* clips: **no tiles at
///   all**, against dense text's 40. That contrast is ADR 0007's whole claim, and it is
///   the reason `rect_clips` is a field of the archetype.
/// - **clip mountain** — twelve hundred rectangular clips resolve to twelve hundred
///   distinct regions and cost **nothing else**: no tile, no layer, nothing culled. The
///   800 atlas keys are the 1 200 placements over 200 outlines collapsing by phase.
/// - **giant** — 1 500 commands, each its own outline and its own key: reuse of exactly
///   one, so the atlas answers nothing and every command rasterises. Against dense
///   text's 5.3 placements per outline, this is the other end of the corpus.
/// - **drawing** — the same "reuse of exactly one" as giant: 1 200 commands over 1 200
///   outlines. **1 194 keys and 6 tiles** is the six strokes, which have no atlas at all
///   — a stroke's coverage is its *expansion*, not its outline, so it goes to the sheet
///   whatever its size — and the 1 194 fills, whose three-pixel tiles the atlas takes.
///   The caller's page has exactly that split, six strokes among 58 009 commands.
///   Otherwise the counters cannot tell this page from giant, and that is worth saying
///   rather than hiding: what differs is the *segments* behind the numbers — 62 400
///   against giant's 12 000, for a ninth of the tile area — and `Counters` has no field
///   for it. So this row gates that the page still draws and still caches; what it
///   exists for is the cost shape, which `examples/encode_threads.rs` measures.
const BASELINE: [(&str, [u32; 9]); 7] = [
    ("median page", [12, 0, 9, 12, 0, 0, 0, 0, 0]),
    ("dense text", [4320, 0, 818, 2164, 1, 40, 0, 2, 0]),
    ("artwork", [684, 0, 300, 300, 1, 600, 3, 185, 0]),
    ("image page", [232, 0, 60, 158, 4, 0, 0, 0, 0]),
    ("clip mountain", [1200, 0, 200, 800, 1200, 0, 0, 0, 0]),
    ("giant", [1500, 0, 1500, 1500, 0, 0, 0, 0, 0]),
    ("drawing", [1200, 0, 1200, 1194, 0, 6, 0, 0, 0]),
];

fn signature(counters: &Counters) -> [u32; 9] {
    [
        counters.commands,
        counters.commands_culled,
        counters.distinct_outlines,
        counters.atlas_distinct_keys,
        counters.clip_distinct_regions,
        counters.tiles,
        counters.layer_textures,
        counters.clip_residue_regions,
        counters.clip_residue_tiles,
    ]
}

/// The gate: what each archetype costs, in quantities that cannot flake.
#[test]
fn the_archetypes_cost_what_they_are_recorded_to_cost() {
    for shape in ARCHETYPES {
        let mut device = cold_device();
        let scene = build(&mut device, shape);
        let (counters, elapsed) = render(&mut device, &scene);
        let Some((_, expected)) = BASELINE.iter().find(|(name, _)| *name == shape.name) else {
            panic!("no baseline recorded for {}", shape.name)
        };
        eprintln!(
            "{:14} {:?} in {elapsed:?}",
            shape.name,
            signature(&counters)
        );
        assert_eq!(
            signature(&counters),
            *expected,
            "{} changed shape: (commands, culled, outlines, atlas keys, clip regions, \
             tiles, layer textures). Every one is an exact function of the scene and the \
             viewport, so this is a change in what the library does — explain it, then \
             record it",
            shape.name
        );
    }
}

/// The scenes are what the profile says they are: the generator cannot drift from the
/// numbers it was built from without this failing.
#[test]
fn the_generator_builds_the_shape_the_profile_states() {
    for shape in ARCHETYPES {
        let mut device = cold_device();
        let scene = build(&mut device, shape);
        let cost = scene.cost();
        // `Cost::commands` walks the tree and counts a group as a node of its own,
        // where `Counters::commands` counts the scene's top level. The two differ by
        // exactly the groups, and both are right about what they measure.
        assert_eq!(
            cost.commands as u32,
            shape.commands + shape.images + shape.groups,
            "{} must hold the commands its profile states",
            shape.name
        );
        assert_eq!(
            cost.clips as u32, shape.clips,
            "{} must define the clip regions its profile states",
            shape.name
        );
        assert!(
            cost.group_depth as u32 <= 1,
            "{}: the corpus reaches depth 2 at most, so a deeper generator is a bug here",
            shape.name
        );
    }
}

/// What the archetypes take on a cold device, printed always and gated loosely.
///
/// **Ignored by default, and that is the honest status.** A wall clock on a loaded
/// machine is context rather than evidence: this file's counter gate passed unchanged
/// under a load average of 32 while this test read sixteen seconds for an archetype
/// recorded at 1.79 — the same code, the same scene, a neighbour compiling something.
/// A gate that fails for that reason teaches people to ignore failures. Run it
/// deliberately, on a quiet machine, when a number is what you want:
/// `cargo test --release -p quorra-gpu --test archetypes -- --ignored --nocapture`. Measured on llvmpipe,
/// release, cold device, 2026-08-12: median page 18 ms, dense text 41, artwork 160,
/// image page 29, clip mountain 30, giant 27. Software rasterisation dominates every
/// one of them, which is why the gate is a multiple rather than a bound.
#[test]
#[ignore = "a wall clock is a measurement here, not a gate; see the doc comment"]
fn no_archetype_takes_absurdly_long() {
    // The rasteriser is a byte loop, so an unoptimised build is an order of magnitude
    // slower and one threshold cannot serve both. Measured 2026-08-12 on llvmpipe, cold
    // device, whole frame including readback: release, the worst archetype is **artwork
    // at 160 ms**; debug, **dense text at 1.79 s**. Each build gets ~4× its own worst.
    let limit = if cfg!(debug_assertions) {
        Duration::from_secs(8)
    } else {
        Duration::from_millis(700)
    };
    for shape in ARCHETYPES {
        let mut device = cold_device();
        let scene = build(&mut device, shape);
        let (_, elapsed) = render(&mut device, &scene);
        eprintln!("{:14} cold frame {elapsed:?}", shape.name);
        assert!(
            elapsed < limit,
            "{} took {elapsed:?}, past {limit:?} — the recorded numbers are in this \
             file's doc comment, and none of them is within a factor of five of this",
            shape.name
        );
    }
}
