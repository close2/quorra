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

/// The device side of the `i`th outline: five sizes about `shape.side`, so that a page's
/// marks are not all one box. **The one statement of it** — the clip cutter below sizes a
/// clip from the marks it will clip, and a second copy of this arithmetic is how the two
/// come apart.
fn outline_side(shape: &Archetype, i: u32) -> f32 {
    shape.side * (1.0 + (i % 5) as f32 * 0.05)
}

/// The archetype's resources: outlines to place, and one image if it has any.
fn upload(
    device: &mut Device,
    shape: &Archetype,
) -> (Vec<OutlineId>, Option<quorra_scene::ImageId>) {
    let outlines = (0..shape.distinct.max(1))
        .map(|i| {
            device
                .upload_outline(&outline_of(shape.segments, outline_side(shape, i)))
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

/// Which clip the `index`th command draws under: a run of consecutive marks in reading
/// order, so that a clip and the marks it clips are in the same part of the page.
///
/// **This is the fixture's whole subject on a curve-clipped page** and it is why the
/// mapping is stated here rather than written inline at its one call site. A clip that
/// does not overlap the marks it clips exercises nothing: before 2026-08-17 this file
/// placed a clip at `position(j, side × 6)` and its marks at `position(i, side)`, two
/// grids of different step, and **0 of dense text's 40 and 8 of artwork's 600** clipped
/// commands had a mark that met the clip clipping it. The rows still read 40 and 600
/// tiles, because a mark whose chain admits nothing was rasterised anyway and multiplied
/// by a residue of zero — so the signature looked like it gated the residue lane through
/// ADR 0049 and ADR 0057, and did not (`doc/notes-tiling-bound.md` §3).
fn clip_of(shape: &Archetype, index: u32) -> usize {
    if shape.clipped == 0 {
        return 0;
    }
    // `index < clipped`, so this is `< clips`: every clipped command names a clip the
    // archetype defines, and the last clip is named by the last run.
    ((u64::from(index) * u64::from(shape.clips)) / u64::from(shape.clipped)) as usize
}

/// The device box the marks under clip `j` occupy — the generator's own arithmetic, with
/// nothing of the crate in it.
///
/// `outline_of` traces an ellipse `side` across and `1.3 × side` tall about the origin,
/// and its cubics' control points are interpolations between points on it, so every
/// point of the curve and of its hull is inside that box. A stroke's expansion reaches
/// half its width outside — which is deliberate: it means a stroked mark under one of
/// these clips is genuinely cut at its rim rather than merely admitted.
fn marks_box(shape: &Archetype, j: usize) -> Option<(f32, f32, f32, f32)> {
    let mut box_of: Option<(f32, f32, f32, f32)> = None;
    for index in 0..shape.clipped {
        if clip_of(shape, index) != j {
            continue;
        }
        let at = position(index, shape.side);
        let side = outline_side(shape, index % shape.distinct.max(1));
        let (hx, hy) = (side * 0.5, side * 0.65);
        box_of = Some(box_of.map_or(
            (at.e - hx, at.f - hy, at.e + hx, at.f + hy),
            |(x0, y0, x1, y1)| {
                (
                    x0.min(at.e - hx),
                    y0.min(at.f - hy),
                    x1.max(at.e + hx),
                    y1.max(at.f + hy),
                )
            },
        ));
    }
    box_of
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
///
/// **A curve clip is cut around the run of marks that draw under it** ([`clip_of`]): the
/// ellipse is scaled onto their box, so it is three or four marks across, a fraction of
/// the page, and it cuts every mark under it — which is the `q W n` shape ADR 0049 and
/// ADR 0057 both exist for, and the one a clip on a grid of its own never was.
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
                curve_clip(shape, i)
            };
            builder
                .clip(outline, transform, FillRule::NonZero, None)
                .unwrap()
        })
        .collect()
}

/// The transform that puts clip `i`'s ellipse over the marks it clips.
///
/// A clip nothing draws under keeps the old placement: it marks nothing either way, and
/// a box computed from an empty set is not a box.
fn curve_clip(shape: &Archetype, i: u32) -> Affine {
    let Some((x0, y0, x1, y1)) = marks_box(shape, i as usize) else {
        return position(i, shape.side * 6.0);
    };
    let side = outline_side(shape, i % shape.distinct.max(1));
    Affine::scale((x1 - x0) / side, (y1 - y0) / (side * 1.3))
        .then(Affine::translate((x0 + x1) * 0.5, (y0 + y1) * 0.5))
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
    let clip = (index < shape.clipped && !clips.is_empty())
        .then(|| clips[clip_of(shape, index).min(clips.len() - 1)]);
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
/// textures, residue regions, residue tiles, coverage texels)`. Every field is an exact
/// function of the scene and the viewport, so these compare by equality on any machine
/// and any adapter. Recorded 2026-08-12; the residue pair joined it on 2026-08-15
/// (ADR 0049) and the coverage texels on 2026-08-17 (ADR 0057).
///
/// **The dense-text and artwork rows were re-taken on 2026-08-17 and are not comparable
/// with the rows before them**, because the page is not the same page: the curve clips
/// were cut around the marks they clip ([`clip_of`], [`curve_clip`]), where before they
/// sat on a grid of their own and met almost nothing. Every other row is untouched, and
/// no library behaviour changed with it — `doc/notes-clipped-instrument.md` §3.
///
/// **`coverage texels` is what the tiles on the sheet hold**, and it is here because two
/// rounds in a row measured a change this signature could not see: `tiles` is a count of
/// tiles and says nothing about how large one was, so a page whose coverage fell by 402×
/// printed the same row as before. It is a count, not a ratio (CLAUDE.md).
///
/// - **median page** — twelve fills over nine outlines at twelve distinct sub-pixel
///   phases, so twelve keys; all cached, so no tile touches the sheet.
/// - **dense text** — 4 320 placements over 818 outlines collapse to 2 164 keys, which
///   is the quantised phase doing its job.
///
///   **40 tiles and 8 956 coverage texels are its two curve clips, cut around their
///   marks.** Each clip takes a run of twenty consecutive marks, so its box is twenty
///   cells wide and one tall — larger than any mark under it, a thirtieth of the page —
///   and every one of the forty clipped commands rasterises a tile that the residue then
///   multiplies into. **No region is kept**, and that is ADR 0049's admission rule
///   working rather than failing: the chain's box costs more than the twenty small tiles
///   it would serve, which is the clause of that ADR written for exactly this shape — a
///   `q W n` around a line of text. So the residue is rasterised once per asking tile,
///   forty times, and `clip_residue_tiles` says so. (Before 2026-08-17 this row read
///   40 tiles and **2 regions** for a page whose clips met **0 of 40** marks: the clips
///   were on a grid of their own, the tiles were rasterised and multiplied by zero, and
///   the two regions served nothing. ADR 0057 made that visible by taking the tiles away;
///   the re-cut makes the row mean what it says.)
/// - **artwork** — 684 top-level nodes are 676 draws plus 8 groups (`Counters::commands`
///   counts the scene's top level). **3 layer textures** are the root's accumulator, one
///   group's at a time, and the copy of the pixels that group's composite covers —
///   ADR 0020's depth pricing showing its work on eight sibling groups, at ADR 0038's one
///   texture per plan. It was 4 while a plan ping-ponged between two.
///
///   **600 tiles, 3 542 360 coverage texels, and both halves of ADR 0049 on one page.**
///   Every one of the 600 curve-clipped commands meets its clip and rasterises a tile of
///   about 5 900 texels; of the 185 chains, **66 keep a region** — cut around three or
///   four marks in one line, it costs less than the tiles it serves — and the rest are
///   refused one and rasterise per tile, **384** times, which is the wrapped runs whose
///   box is the width of the page's grid. 66 + 384 = 450 rasterisations where the page
///   has 600 clipped commands, and that difference is what ADR 0049 buys, measured on a
///   page where the clips actually clip. (This row read **8 tiles, 2 regions, 6 residue
///   tiles, 12 284 texels** until 2026-08-17, and 600 tiles / 185 regions / 0 residue
///   tiles before ADR 0057 — none of the three is comparable with the others, because the
///   first two are a page whose 185 clips met 8 of its 600 marks.)
/// - **image page** — 200 fills and 32 images under *rectangular* clips: **no tiles at
///   all**. Where dense text's clips leave a residue this one's resolve to a rectangle,
///   which is ADR 0007's whole claim and the reason `rect_clips` is a field of the
///   archetype.
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
///   exists for is the cost shape, which `examples/encode_threads.rs` measures. Its
///   **245 coverage texels** are those six strokes' expansions and nothing else.
const BASELINE: [(&str, [u64; 10]); 7] = [
    ("median page", [12, 0, 9, 12, 0, 0, 0, 0, 0, 0]),
    ("dense text", [4320, 0, 818, 2164, 1, 40, 0, 0, 40, 8_956]),
    ("artwork", [684, 0, 300, 300, 1, 600, 3, 66, 384, 3_542_360]),
    ("image page", [232, 0, 60, 158, 4, 0, 0, 0, 0, 0]),
    ("clip mountain", [1200, 0, 200, 800, 1200, 0, 0, 0, 0, 0]),
    ("giant", [1500, 0, 1500, 1500, 0, 0, 0, 0, 0, 0]),
    ("drawing", [1200, 0, 1200, 1194, 0, 6, 0, 0, 0, 245]),
];

fn signature(counters: &Counters) -> [u64; 10] {
    [
        u64::from(counters.commands),
        u64::from(counters.commands_culled),
        u64::from(counters.distinct_outlines),
        u64::from(counters.atlas_distinct_keys),
        u64::from(counters.clip_distinct_regions),
        u64::from(counters.tiles),
        u64::from(counters.layer_textures),
        u64::from(counters.clip_residue_regions),
        u64::from(counters.clip_residue_tiles),
        counters.coverage.texels,
    ]
}

/// The gate: what each archetype costs, in quantities that cannot flake.
///
/// **Every archetype is measured before any of them is judged.** A loop that asserts as
/// it goes reports the first row that moved and hides the rest, which is the wrong shape
/// for a signature: a change that moves one row and a change that moves five are
/// different changes, and the second must not read as the first.
#[test]
fn the_archetypes_cost_what_they_are_recorded_to_cost() {
    let mut moved = Vec::new();
    for shape in ARCHETYPES {
        let mut device = cold_device();
        let scene = build(&mut device, shape);
        let (counters, elapsed) = render(&mut device, &scene);
        let Some((_, expected)) = BASELINE.iter().find(|(name, _)| *name == shape.name) else {
            panic!("no baseline recorded for {}", shape.name)
        };
        let actual = signature(&counters);
        eprintln!("{:14} {actual:?} in {elapsed:?}", shape.name);
        if actual != *expected {
            moved.push(format!("{}: {actual:?} against {expected:?}", shape.name));
        }
    }
    assert!(
        moved.is_empty(),
        "the archetype signature moved — (commands, culled, outlines, atlas keys, clip \
         regions, tiles, layer textures, residue regions, residue tiles, coverage \
         texels). Every one is an exact function of the scene and the viewport, so this \
         is a change in what the library does — explain it, then record it:\n  {}",
        moved.join("\n  ")
    );
}

/// **A curve clip clips the marks under it** — asserted as an interaction, in the two
/// quantities that say the interaction happened, on both sides of the library's boundary.
///
/// The trap this exists for is written down in `doc/notes-tiling-bound.md` §3 and cost
/// two ADRs: a fixture whose subject is an *interaction* needs a gate that fails when the
/// interaction stops happening, and the signature above was not one. It counted 40 and
/// 600 tiles for two pages whose clips and marks did not overlap at all, because until
/// ADR 0057 a mark whose chain admitted nothing still got a mark-sized tile — so the
/// count survived the property it was standing in for.
///
/// Two assertions, each from a different side:
///
/// - **from the generator's own arithmetic**, with nothing of the crate in it: every one
///   of the `clipped` commands has a mark box that meets the box of the clip that clips
///   it. This is the property [`curve_clip`] is written to hold.
/// - **from the counters**: `tiles == clipped`. A mark whose chain admits no pixel is not
///   rasterised (ADR 0057), so the library agreeing that there are exactly `clipped`
///   tiles is the same statement measured through the encode. It is exact and
///   adapter-independent, like every other row of the signature.
///
/// The residue lane's own numbers — how many chains were rasterised once over a region
/// and how many per tile — are in `BASELINE`, which compares by equality; what is
/// asserted here is only that a rasterisation happened at all and that none of them is
/// unaccounted for.
#[test]
fn a_curve_clip_clips_the_marks_that_draw_under_it() {
    for shape in ARCHETYPES {
        if shape.rect_clips || shape.clips == 0 || shape.clipped == 0 {
            continue;
        }
        let meeting = (0..shape.clipped)
            .filter(|index| {
                // The clip's box as the *scene* will carry it: `curve_clip`'s transform
                // applied to the ellipse `outline_of` traces. Deliberately not
                // `marks_box` — that is the box `curve_clip` was built from, so testing
                // against it would assert an identity and pass however the clip is
                // placed. (It does: the first version of this gate was tautological and
                // survived the forced defect, and only the counter below caught it.)
                let clip = clip_of(shape, *index) as u32;
                let placed = curve_clip(shape, clip);
                let clip_side = outline_side(shape, clip % shape.distinct.max(1));
                let (chx, chy) = (placed.a * clip_side * 0.5, placed.d * clip_side * 0.65);
                let at = position(*index, shape.side);
                let side = outline_side(shape, index % shape.distinct.max(1));
                let (hx, hy) = (side * 0.5, side * 0.65);
                at.e - hx < placed.e + chx
                    && at.e + hx > placed.e - chx
                    && at.f - hy < placed.f + chy
                    && at.f + hy > placed.f - chy
            })
            .count() as u32;
        assert_eq!(
            meeting, shape.clipped,
            "{}: the fixture's own arithmetic says only {meeting} of its {} clipped \
             commands have a mark that meets the clip clipping it — a clip that admits \
             nothing exercises nothing, whatever the tile count says",
            shape.name, shape.clipped,
        );

        let mut device = cold_device();
        let scene = build(&mut device, shape);
        let (counters, _) = render(&mut device, &scene);
        assert_eq!(
            counters.tiles, shape.clipped,
            "{}: {} clipped commands and {} coverage tiles. Every mark under a curve clip \
             rasterises one and nothing else on this page does, so a difference is a \
             clipped mark whose chain admits no pixel of it (ADR 0057)",
            shape.name, shape.clipped, counters.tiles,
        );
        assert!(
            counters.clip_residue_regions + counters.clip_residue_tiles <= shape.clipped,
            "{}: {} residue rasterisations for {} clipped commands — a chain is \
             rasterised once over its region or once per asking tile (ADR 0049), never \
             both",
            shape.name,
            counters.clip_residue_regions + counters.clip_residue_tiles,
            shape.clipped,
        );
        assert!(
            counters.clip_residue_regions + counters.clip_residue_tiles > 0,
            "{}: the page draws under curve clips and rasterised no residue at all",
            shape.name,
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
/// `cargo test --release -p quorra-gpu --test archetypes -- --ignored --nocapture`.
/// Measured on llvmpipe, release, cold device, **2026-08-17 at load average 4.95, with
/// the curve clips cut around their marks**: median page 20 ms, dense text 44, artwork
/// **148**, image page 29, clip mountain 30, giant 36, drawing 46. Software rasterisation
/// dominates every one of them, which is why the gate is a multiple rather than a bound.
///
/// The same list on 2026-08-12 read 18 / 41 / **160** / 29 / 30 / 27 on a page whose
/// clips met 8 of the 600 marks they clipped — nearly the same numbers for a page doing
/// nearly the same work to no effect, which is what makes the two look comparable when
/// they are not. The same run at load average 380 read artwork at **605 ms**, four times
/// the quiet figure, which is the whole argument for `#[ignore]`.
#[test]
#[ignore = "a wall clock is a measurement here, not a gate; see the doc comment"]
fn no_archetype_takes_absurdly_long() {
    // The rasteriser is a byte loop, so an unoptimised build is an order of magnitude
    // slower and one threshold cannot serve both. Re-measured 2026-08-17 on llvmpipe, cold
    // device, whole frame including readback, load average 5–8, with the curve clips cut
    // around their marks: release, the worst archetype is **artwork at 148 ms**; debug,
    // **dense text at 2.29 s** (artwork 0.76 s). Each build gets ~4× its own worst, and
    // both thresholds are unchanged by the re-cut — artwork does about the work it did
    // before ADR 0057 took its empty tiles away, which is why the release figure moved by
    // 12 ms and not by a factor.
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
