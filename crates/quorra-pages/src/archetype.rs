//! What an archetype page *is*, and the arithmetic that places its marks.
//!
//! Every function here is a pure function of the archetype and an index: no device, no
//! randomness and no clock, so the same archetype produces the same scene on every run
//! and every machine. That is what lets `tests/archetypes.rs` compare a whole counter
//! row by equality rather than by threshold.

// A page's coordinates are bounded by its own size, and every cast below is an index or
// a device coordinate inside it. The lint policy matches the fixture files this crate
// replaces; nothing here is derived from a document.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use quorra_scene::{Affine, Point, Segment};

/// One page shape, as a set of measured counts.
///
/// Every field is a number from `doc/corpus-profile.md`; the geometry that realises
/// them is this crate's. A page is identified by its [`name`](Archetype::name), and two
/// pages that differ in any field are two pages — which is why [`DENSE_TEXT_UNCLIPPED`]
/// exists beside [`DENSE_TEXT`] rather than pretending to be it.
///
/// [`DENSE_TEXT`]: crate::DENSE_TEXT
/// [`DENSE_TEXT_UNCLIPPED`]: crate::DENSE_TEXT_UNCLIPPED
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Archetype {
    /// The page's name, and the key its recorded row is found under.
    pub name: &'static str,
    /// Device width the page is laid out for.
    pub width: u32,
    /// Device height the page is laid out for.
    pub height: u32,
    /// Drawing commands, groups excluded.
    pub commands: u32,
    /// Distinct outlines uploaded — `commands / distinct` is the reuse the atlas sees.
    pub distinct: u32,
    /// Segments per outline, as a closed curve.
    pub segments: u32,
    /// Device side of one shape, in pixels at 1×.
    pub side: f32,
    /// How many of the commands are strokes rather than fills.
    pub strokes: u32,
    /// Image placements the page makes.
    pub images: u32,
    /// The side of the synthetic image behind those placements.
    pub image_side: u32,
    /// Clip regions defined.
    pub clips: u32,
    /// How many commands draw under one.
    pub clipped: u32,
    /// Whether those clips are axis-aligned rectangles, which ADR 0007 resolves to a
    /// rectangle at encode time, or curves, which leave a residue every clipped command
    /// must multiply into a coverage tile. The two are different lanes and the corpus
    /// has both.
    pub rect_clips: bool,
    /// Groups wrapping runs of the commands.
    pub groups: u32,
    /// How many of those groups carry a blend mode.
    pub blended_groups: u32,
    /// What the page is recorded to cost, or `None` for a page no gate has priced.
    pub recorded: Option<Recorded>,
}

/// What a page is recorded to cost, in quantities that are exact functions of the scene
/// and the viewport — so they compare by equality on any machine and any adapter.
///
/// **Named fields rather than an array, on purpose.** Each consumer builds one of these
/// from its own `Counters` (which lives in `quorra-gpu`, a crate this one must not
/// depend on), and a field name is a mapping that cannot be written wrongly in silence
/// the way a positional row can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recorded {
    /// `Counters::commands` — the scene's top level, groups counted as one node each.
    pub commands: u64,
    /// `Counters::commands_culled`.
    pub commands_culled: u64,
    /// `Counters::distinct_outlines`.
    pub distinct_outlines: u64,
    /// `Counters::atlas_distinct_keys`.
    pub atlas_distinct_keys: u64,
    /// `Counters::clip_distinct_regions`.
    pub clip_distinct_regions: u64,
    /// `Counters::tiles`.
    pub tiles: u64,
    /// `Counters::layer_textures`.
    pub layer_textures: u64,
    /// `Counters::clip_residue_regions`.
    pub clip_residue_regions: u64,
    /// `Counters::clip_residue_tiles`.
    pub clip_residue_tiles: u64,
    /// `Counters::coverage.texels`.
    pub coverage_texels: u64,
}

/// A closed curve of `segments` segments about the origin, `side` across: the shape a
/// letterform has for costing purposes — curved, closed, and filled under the non-zero
/// rule.
#[must_use]
pub fn outline_of(segments: u32, side: f32) -> Vec<Segment> {
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
/// shape, so ADR 0007 resolves a clip built from it to a rectangle — no residue, and no
/// coverage tile for the commands under it.
#[must_use]
pub fn rect_path(half: f32) -> Vec<Segment> {
    vec![
        Segment::MoveTo(Point::new(-half, -half)),
        Segment::LineTo(Point::new(half, -half)),
        Segment::LineTo(Point::new(half, half)),
        Segment::LineTo(Point::new(-half, half)),
        Segment::Close,
    ]
}

/// Where the `index`th command lands: a reading-order grid over the page.
#[must_use]
pub fn position(shape: &Archetype, index: u32, side: f32) -> Affine {
    let step = side + 3.5;
    let columns = ((shape.width as f32 - 16.0) / step).max(1.0) as u32;
    let x = 8.0 + (index % columns) as f32 * step + side * 0.5;
    let y = 12.0 + (index / columns) as f32 * (side + 4.25) + side * 0.5;
    Affine::translate(x, y % (shape.height as f32 - 24.0))
}

/// The device side of the `i`th outline: five sizes about `shape.side`, so that a
/// page's marks are not all one box.
///
/// **The one statement of it** — [`curve_clip`] sizes a clip from the marks it will
/// clip, and a second copy of this arithmetic is how the two come apart.
#[must_use]
pub fn outline_side(shape: &Archetype, i: u32) -> f32 {
    shape.side * (1.0 + (i % 5) as f32 * 0.05)
}

/// Which clip the `index`th command draws under: a run of consecutive marks in reading
/// order, so that a clip and the marks it clips are in the same part of the page.
///
/// **This is the fixture's whole subject on a curve-clipped page.** A clip that does not
/// overlap the marks it clips exercises nothing: before 2026-08-17 the generator placed
/// a clip at `position(j, side × 6)` and its marks at `position(i, side)`, two grids of
/// different step, and **0 of dense text's 40 and 8 of artwork's 600** clipped commands
/// had a mark that met the clip clipping it. The rows still read 40 and 600 tiles,
/// because a mark whose chain admits nothing was rasterised anyway and multiplied by a
/// residue of zero — so the signature looked like it gated the residue lane through
/// ADR 0049 and ADR 0057, and did not (`doc/notes-tiling-bound.md` §3).
#[must_use]
pub fn clip_of(shape: &Archetype, index: u32) -> usize {
    if shape.clipped == 0 {
        return 0;
    }
    // `index < clipped`, so this is `< clips`: every clipped command names a clip the
    // archetype defines, and the last clip is named by the last run.
    ((u64::from(index) * u64::from(shape.clips)) / u64::from(shape.clipped)) as usize
}

/// The device box the marks under clip `j` occupy — the generator's own arithmetic, with
/// nothing of the renderer in it.
///
/// [`outline_of`] traces an ellipse `side` across and `1.3 × side` tall about the origin,
/// and its cubics' control points are interpolations between points on it, so every
/// point of the curve and of its hull is inside that box. A stroke's expansion reaches
/// half its width outside — which is deliberate: it means a stroked mark under one of
/// these clips is genuinely cut at its rim rather than merely admitted.
#[must_use]
pub fn marks_box(shape: &Archetype, j: usize) -> Option<(f32, f32, f32, f32)> {
    let mut box_of: Option<(f32, f32, f32, f32)> = None;
    for index in 0..shape.clipped {
        if clip_of(shape, index) != j {
            continue;
        }
        let at = position(shape, index, shape.side);
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

/// The transform that puts clip `i`'s ellipse over the marks it clips: three or four
/// marks across, a fraction of the page, and cutting every one of them.
///
/// A clip nothing draws under keeps the old placement: it marks nothing either way, and
/// a box computed from an empty set is not a box.
#[must_use]
pub fn curve_clip(shape: &Archetype, i: u32) -> Affine {
    let Some((x0, y0, x1, y1)) = marks_box(shape, i as usize) else {
        return position(shape, i, shape.side * 6.0);
    };
    let side = outline_side(shape, i % shape.distinct.max(1));
    Affine::scale((x1 - x0) / side, (y1 - y0) / (side * 1.3))
        .then(Affine::translate((x0 + x1) * 0.5, (y0 + y1) * 0.5))
}
