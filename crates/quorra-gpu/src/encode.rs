//! Phase 1 of a frame: classify into lanes, rasterise coverage, count, then lay out
//! instance data.
//!
//! One CPU walk over the scene's commands (PLAN.md Part 1 §1.2), sorting each into
//! the cheapest lane that draws it exactly:
//!
//! - **rectangle** — axis-aligned rect, axis-preserving transform, fully rectangular
//!   clip: analytic coverage in the shader, clip applied by intersection here
//!   (ADR 0007), zero per-pixel clip cost. A [`Command::Rect`] reaches it, and so does
//!   a fill whose *outline* is four axis-aligned edges, which is the only form a
//!   document rectangle actually arrives in (ADR 0047);
//! - **glyph** — a fill whose device size fits an atlas tile: coverage rasterised
//!   once per `(outline, linear part, quantised phase)` key (ADR 0008/0009) and drawn
//!   as a quad over the persistent R8 atlas;
//! - **path** — everything else that is drawable today: large fills, strokes,
//!   rect-with-residue-clip: coverage rasterised into the frame's scratch image and
//!   drawn as a quad. Non-rectangular clip links multiply into the mask here, which
//!   is M5's residue — the R8 mask the brief said a *rectangular* clip must never
//!   become, applied exactly where it must.
//!
//! **Counting precedes allocation** (§5's first preference): instance buffers are
//! sized from the command count and every rasterised mask is charged against the
//! frame budget before its bytes exist, so there is no fixed-size table for a scene
//! to overflow.
//!
//! Since M6 the walk also builds the **layer tree**: a group becomes a child
//! [`LayerPlan`] composited once under its spec (ISO 32000-2 §11.4.5); an element
//! with a non-`Normal` blend becomes an implicit single-element child, so §11.3.5
//! runs through one compositor; knockout groups and `Compose::Src` elements mark
//! their draws [`DrawStyle::Knockout`] for the two-pass erase/add of ADR 0010; and a
//! used soft mask's group is planned like any layer, for reduction before the frame
//! draws.
//!
//! M7 completes the vocabulary with the **rare-case lanes** (ADR 0011): an image, a
//! ramp shading or a mesh becomes a single uniform-driven quad ([`ImageOp`],
//! [`ShadedOp`]) rather than a fourth instance stream — the brief's §0 premise is
//! that most of a page is glyphs and rectangles, and the encoding matches it.
//!
//! Five of the walk's subsystems are their own concerns and their own modules:
//! [`clips`] turns a chain of [`ClipId`]s into a rectangle and a residue (ADR 0007,
//! ADR 0030), [`residue`] keeps a chain's residue region so that it is rasterised once
//! rather than once per mark (ADR 0049), [`scratch`] is the frame's coverage sheet with
//! the shelf packing that fills it (ADR 0021, ADR 0034), [`rare`] is the image and
//! shading lanes — the quads the brief's §0 calls the rare case (ADR 0011) — and
//! [`hull`] is the memo that bounds a placement by translating its neighbour's box
//! rather than transforming the outline again (ADR 0045), and [`parallel`] is the seam
//! that lets a run of marks rasterise off this thread when the host asked for that. What
//! is left here is the walk itself and the two lanes it exists for.

mod clips;
mod coverage;
mod device_space;
mod encoded;
mod function;
mod hull;
mod instance;
mod layer;
mod parallel;
mod plan;
mod rare;
mod rect;
mod residue;
mod scratch;

#[cfg(test)]
mod tests;

use std::collections::HashSet;

use quorra_scene::{
    Affine, BlendMode, ClipId, Command, Compose, FillRule, MaskId, OutlineId, Paint, Rect, Scene,
};

use clips::{ClipResolver, ResolvedClip, open_clip};
use device_space::{apply, compose, target_rect, tile_side, transform_preserves_axes};
use encoded::finish;
use instance::instance_reserve;
use parallel::{Draw, Job};

use crate::atlas::{AtlasStore, GlyphKey, GlyphPlacement};
use crate::census::Census;
use crate::error::RenderError;
use crate::instrument::EncodeClock;
use crate::keyhash::FastSet;
use crate::raster::{self, DeviceTransform, Rule};
use crate::resources::ResourceStore;
use crate::startup::Coverage;
use crate::viewport::Viewport;

pub(crate) use encoded::Encoded;
pub(crate) use function::{FunctionOp, empty_stack_reports};
pub(crate) use instance::{
    Batch, BatchKind, DrawStyle, QUAD_INSTANCE_STRIDE, RECT_INSTANCE_STRIDE,
};
pub(crate) use layer::{ChildOp, MaskPlan};
pub(crate) use plan::{LayerPlan, Op};
pub(crate) use rare::{ImageOp, PaintSource, ShadedOp};
use residue::ResidueRegions;
pub(crate) use scratch::Scratch;
use scratch::ScratchPacker;

/// One solid fill, resolved as far as the lane choice needs it.
///
/// A struct rather than nine parameters: the arm that draws it asks the cache one
/// question and then hands the same fill to whichever of three lanes answers, and
/// threading the fill's own description through each of them by hand is how two of them
/// come to disagree about it.
struct SolidFill {
    outline: OutlineId,
    /// The scene's transform, which is what the census counted by.
    transform: Affine,
    /// The same transform composed with the viewport, which is what the tile is
    /// rasterised and keyed by.
    to_device: DeviceTransform,
    rule: Rule,
    color: quorra_scene::Color,
    /// Device bounds: min x, min y, max x, max y.
    bounds: (f32, f32, f32, f32),
    style: DrawStyle,
    mask: Option<u32>,
}

/// The linear part of a transform as the bits a census counts by.
///
/// The *scene's* transform, not the device's: the two differ by the viewport, which is
/// one affine for the whole frame, so equal scene linear parts compose to equal device
/// ones and the census can be taken before a viewport is in hand.
fn linear_bits(transform: Affine) -> [u32; 4] {
    [
        transform.a.to_bits(),
        transform.b.to_bits(),
        transform.c.to_bits(),
        transform.d.to_bits(),
    ]
}

/// The encoder's working state for one frame walk.
struct Encoder<'a> {
    scene: &'a Scene,
    viewport: &'a Viewport<'a>,
    /// [`target_rect`] of `viewport`, computed once: every command is tested against
    /// it before its geometry is built.
    visible: Rect,
    /// Which lane makes coverage bytes (ADR 0016).
    coverage: Coverage,
    /// The GPU lane's triangles and tiles, empty under [`Coverage::Cpu`].
    winding: crate::winding::Sheet,
    /// How often the scene places each shape, taken before the walk (ADR 0029).
    census: Census,
    resources: &'a ResourceStore,
    atlas: &'a mut AtlasStore,
    quantum: Option<u16>,
    clips: ClipResolver,
    /// A clip chain's residue coverage, rasterised once over the chain's own region
    /// wherever that is worth more than the tiles it replaces (ADR 0049).
    ///
    /// Counted before the walk, like [`Census`] above and for the same reason: what a
    /// chain's region is worth depends on how many commands will ask for it, which is not
    /// knowable from the one that asks first. Unlike the census this is two integer passes
    /// with no hashing, and every lane reads it.
    residue: ResidueRegions,
    scratch: ScratchPacker,
    rect_instances: Vec<u8>,
    quad_instances: Vec<u8>,
    /// The plan under construction: `usize::MAX` is the root, anything else an
    /// index into `layers`.
    current_plan: usize,
    root: LayerPlan,
    layers: Vec<LayerPlan>,
    mask_plans: Vec<Option<MaskPlan>>,
    /// The active drawing style, set by the enclosing knockout group.
    style: DrawStyle,
    budget: u64,
    spent: u64,
    /// The `distinct_outlines` counter's working set. [`FastSet`] and not the standard
    /// library's, because this is asked **once per fill** — the same position as the
    /// atlas key maps, and for the same reason `keyhash` exists: a `u32` an encoder
    /// produced is not an adversary's key, and `SipHash`'s per-process seed is a
    /// liability rather than a defence here. Only `len()` is ever read, so the iteration
    /// order this changes reaches nothing. Measured on the dense-text archetype (4 320
    /// fills, callgrind, `doc/PLAN.md` 2026-08-14): 1.65 M of a 19.70 M-instruction
    /// encode was `SipHash` on this one set, and swapping the hasher took **0.56 M**
    /// of it — the remainder is the probing, which is `hashbrown`'s either way.
    distinct_outlines: FastSet<u32>,
    atlas_keys: FastSet<GlyphKey>,
    used_images: HashSet<u32>,
    used_ramps: HashSet<u32>,
    used_meshes: HashSet<u32>,
    used_functions: HashSet<u32>,
    segments: u64,
    /// Commands that could reach no pixel of the target, and so were never built.
    culled: u32,
    /// Child layers whose clip left them no pixel to contribute, and so were never
    /// composited (ADR 0041).
    culled_layers: u32,
    atlas_pressure: bool,
    /// See `Encoded::atlas_requested_bytes`.
    atlas_requested_bytes: u64,
    /// See `Encoded::atlas_entries_used`.
    atlas_entries_used: u32,
    /// Sheet bytes already charged tile by tile, so the sheet's own extent can be
    /// charged once at the end without paying twice (ADR 0021).
    scratch_charged: u64,
    /// What encode spent its time on, when the caller asked (ADR 0023).
    clock: EncodeClock,
    /// How many threads the *host* said the frame's geometry may use
    /// ([`crate::startup::Options::encode_threads`]). One is the default and takes the
    /// walk this encoder has always taken, with no queue and no allocation.
    threads: usize,
    /// Marks whose coverage has not been made yet, in encounter order ([`parallel`]).
    /// Always empty when `threads` is one.
    queue: Vec<Job<'a>>,
    /// The atlas keys the queue will write, so that two queued jobs cannot rasterise
    /// one key against an atlas neither has reached (`parallel`'s guard).
    queued_keys: FastSet<GlyphKey>,
    /// The queue's weight in outline segments, against which the fan-out's floor is
    /// tested. A `u64` because it is a sum of scene-derived counts.
    queued_weight: u64,
    /// Host memory the queue holds, bounded above (`Job::held`), against which
    /// `in_flight_limit` is tested.
    queued_bytes: u64,
    /// What that sum may reach before the queue is drained (`parallel::in_flight_limit`).
    in_flight_limit: u64,
    /// One control-hull box per `(outline, linear part)`, so the 3 502 repeats of a
    /// dense page's 818 letterforms add a translation instead of transforming 37 control
    /// points again — 21 % of the encode, and [`hull`] carries the benchmark and the
    /// argument that it changes no bit of any box.
    hulls: hull::HullMemo,
}

/// Walk the scene once: classify, count, rasterise, check the budget, lay out
/// instances.
#[allow(clippy::too_many_arguments)] // the frame's inputs, named once at the one call
pub(crate) fn encode(
    scene: &Scene,
    viewport: &Viewport<'_>,
    frame_budget_bytes: u64,
    max_dimension: u32,
    resources: &ResourceStore,
    atlas: &mut AtlasStore,
    quantum: Option<u16>,
    coverage: Coverage,
    instrument: bool,
    threads: usize,
) -> Result<Encoded, RenderError> {
    let commands = scene.commands();

    // Count, check against the stated budget, then allocate — in that order. Every
    // command costs at most one rect and one quad instance.
    let per_command = RECT_INSTANCE_STRIDE.saturating_add(QUAD_INSTANCE_STRIDE);
    let needed = (commands.len() as u64).saturating_mul(per_command);
    if needed > frame_budget_bytes {
        return Err(RenderError::FrameBudgetExceeded {
            needed,
            budget: frame_budget_bytes,
        });
    }

    let mut encoder = Encoder {
        scene,
        viewport,
        visible: target_rect(viewport),
        coverage,
        winding: crate::winding::Sheet::default(),
        // One pass over the commands before the walk: the lane a fill takes depends on
        // how many *other* fills share its tile, which is not knowable from the fill
        // (ADR 0029).
        //
        // **Only the GPU lane reads it**, and `take_gpu_lane` answers `false` on sight
        // under `Coverage::Cpu` — so the caller's default configuration must not pay for
        // the walk. Measured at 25 µs on a 5 933-command page against an encode of 80,
        // which is a quarter of a phase this project measures in microseconds. An empty
        // census answers "not placed once" to every shape, which is the lane every fill
        // would have taken anyway.
        census: match coverage {
            Coverage::Gpu => Census::of(scene),
            Coverage::Cpu => Census::default(),
        },
        resources,
        atlas,
        quantum,
        clips: ClipResolver::new(scene.clips().len()),
        residue: ResidueRegions::of(scene, residue::budget(frame_budget_bytes)),
        // The scratch sheet spans the full device dimension both ways: its *byte*
        // cost is charged tile by tile against the frame budget, so the dimension
        // is capacity, not commitment — and a 2048-texel width refused real pages
        // whose coverage was well inside the budget (QUORRA_FEEDBACK.md §3).
        scratch: ScratchPacker::new(max_dimension, max_dimension),
        // Sized from the count that was just checked, rather than grown into. The
        // budget above has already *charged* one rect and one quad per command, so
        // reserving that much allocates exactly what the frame was priced at — which
        // is §5's "count then allocate" taken literally, where growing is the same
        // bytes reached through a dozen reallocations. It over-reserves whichever of
        // the two lanes a page does not use (a page of glyphs never writes a rect
        // instance), and that is the stated cost: virtual bytes already inside the
        // budget, never touched, against the growth measured on the dense-text
        // archetype at 0.13 M of a 19.14 M-instruction encode (callgrind,
        // `doc/PLAN.md` 2026-08-14).
        rect_instances: Vec::with_capacity(instance_reserve(commands.len(), RECT_INSTANCE_STRIDE)),
        quad_instances: Vec::with_capacity(instance_reserve(commands.len(), QUAD_INSTANCE_STRIDE)),
        current_plan: usize::MAX,
        root: LayerPlan::default(),
        layers: Vec::new(),
        mask_plans: (0..scene.masks().len()).map(|_| None).collect(),
        style: DrawStyle::Over,
        budget: frame_budget_bytes,
        spent: needed,
        distinct_outlines: FastSet::default(),
        atlas_keys: FastSet::default(),
        used_images: HashSet::new(),
        used_ramps: HashSet::new(),
        used_meshes: HashSet::new(),
        used_functions: HashSet::new(),
        segments: 0,
        culled: 0,
        culled_layers: 0,
        atlas_pressure: false,
        atlas_requested_bytes: 0,
        atlas_entries_used: 0,
        scratch_charged: 0,
        clock: EncodeClock::new(instrument),
        hulls: hull::HullMemo::default(),
        threads,
        queue: Vec::new(),
        queued_keys: FastSet::default(),
        queued_weight: 0,
        queued_bytes: 0,
        in_flight_limit: parallel::in_flight_limit(frame_budget_bytes),
    };

    for (index, command) in commands.iter().enumerate() {
        encoder.command(index, command)?;
    }
    // The last run of marks, which no later command drained. Everything below reads the
    // sheet, the budget and the plans, and every one of those is what the queue has yet
    // to touch.
    encoder.drain_queue()?;

    finish(encoder, commands.len())
}

impl Encoder<'_> {
    // `index` names commands in refusals; with only M7's images left to refuse it
    // currently reaches errors only through nested walks, which the lint misreads.
    #[allow(clippy::only_used_in_recursion)]
    fn command(&mut self, index: usize, command: &Command) -> Result<(), RenderError> {
        match command {
            Command::Rect {
                rect,
                transform,
                color,
                clip,
                mask,
            } => self.encode_rect(*rect, *transform, *color, *clip, *mask),
            Command::Fill {
                outline,
                transform,
                rule,
                paint,
                clip,
                blend,
                compose: compose_mode,
                mask,
            } => self.encode_fill(
                *outline,
                *transform,
                *rule,
                *paint,
                *clip,
                *blend,
                *compose_mode,
                *mask,
            ),
            Command::Stroke {
                outline,
                transform,
                stroke,
                paint,
                clip,
                blend,
                mask,
            } => self.encode_stroke(
                index, *outline, *transform, *stroke, *paint, *clip, *blend, *mask,
            ),
            Command::Image {
                image,
                transform,
                alpha,
                filter,
                clip,
                blend,
                mask,
            } => self.encode_image(*image, *transform, *alpha, *filter, *clip, *blend, *mask),
            Command::Group { spec, commands } => self.encode_group(spec, commands),
        }
    }

    /// The stroke arm: expansion via the path lane, non-Normal blends through an
    /// implicit child (as in `encode_fill`).
    #[allow(clippy::too_many_arguments)] // one command's fields, destructured once
    fn encode_stroke(
        &mut self,
        index: usize,
        outline: OutlineId,
        transform: Affine,
        stroke: quorra_scene::Stroke,
        paint: Paint,
        clip: Option<ClipId>,
        blend: BlendMode,
        mask: Option<MaskId>,
    ) -> Result<(), RenderError> {
        let mask = self.use_mask(mask)?;
        let stored = self
            .resources
            .outline(outline)
            .ok_or(RenderError::UnknownOutline { outline })?;
        let to_device = compose(transform, self.viewport);
        let resolved = self.resolve_clip(clip)?;
        // Visibility before the blend wrap, so a stroke outside the target costs
        // neither its expansion nor the implicit group §11.3.5 would put it in. The
        // outline's hull grows by the stroke's own reach: the width is device-space
        // (§4.5 resolved it per placement), a miter join may carry a corner half the
        // width times the limit away from it (§8.4.3.5), and a cap extends half the
        // width — which a limit of at least 1 already covers.
        let reach = stroke.width * 0.5 * stroke.miter_limit;
        if let Some((x0, y0, x1, y1)) = self.hulls.bounds(outline, &stored.segments, &to_device)
            && self.culled((x0 - reach, y0 - reach, x1 + reach, y1 + reach), &resolved)
        {
            return Ok(());
        }
        if blend != BlendMode::Normal && self.style == DrawStyle::Over {
            // §11.3.5 for a single element: an implicit one-element group (the same
            // degeneracy argument as in `encode_fill` skips it under knockout).
            let child = self.plan_child(|encoder| {
                let plain = Command::Stroke {
                    outline,
                    transform,
                    stroke,
                    paint,
                    clip,
                    blend: BlendMode::Normal,
                    mask: None,
                };
                encoder.command(index, &plain)
            })?;
            self.push_op(Op::Child(ChildOp::implicit_blend_group(child, blend, mask)))?;
            return Ok(());
        }
        self.distinct_outlines.insert(outline.0);
        self.segments = self.segments.saturating_add(stored.segments.len() as u64);
        // A solid stroke is expansion and a fill, both of them pure functions of this
        // command's own outline, so it takes the same seam a fill does (`parallel`).
        if let Paint::Solid(color) = paint
            && let Some(rect) = self.deferrable_bounds(&resolved)
        {
            // The hull grown by the stroke's own reach, which is the box the cull above
            // already tested and so an upper bound on the expansion's device bounds.
            let bound = self
                .hulls
                .bounds(outline, &stored.segments, &to_device)
                .map_or(0, |(x0, y0, x1, y1)| {
                    self.tile_bound((x0 - reach, y0 - reach, x1 + reach, y1 + reach), &resolved)
                });
            return self.enqueue(Job::sheet(
                &stored.segments,
                to_device,
                Some(stroke),
                Rule::NonZero,
                rect,
                bound,
                Draw::new(color, resolved.rect, self.style, mask),
            ));
        }
        // Flatten under the full transform, then expand: the width arrived
        // resolved (§4.5), so our job is caps, joins and miters only.
        let span = self.clock.start();
        let polylines = raster::flatten(&stored.segments, to_device);
        let stroked = raster::stroke_polylines(&polylines, stroke);
        self.clock.geometry(span);
        match paint {
            Paint::Solid(color) => {
                self.push_coverage(&stroked, Rule::NonZero, color, &resolved, mask)
            }
            // Every non-solid paint is one quad over the stroke's coverage: which one it
            // is decided by `rare_paint`, and nothing about the difference reaches here.
            Paint::Shading { .. } | Paint::Mesh(_) | Paint::Function { .. } => {
                let Some(rare) = self.rare_paint(paint)? else {
                    return Ok(());
                };
                let style = self.style;
                self.push_rare_coverage(rare, &stroked, Rule::NonZero, &resolved, style, mask)
            }
        }
    }

    /// The fill arm of the command walk: pick the glyph or path lane by device size
    /// and residue state; route non-Normal blends through an implicit child layer;
    /// mark `Compose::Src` for the knockout two-pass (§4.1).
    #[allow(clippy::too_many_arguments)] // one command's fields, destructured once
    fn encode_fill(
        &mut self,
        outline: OutlineId,
        transform: Affine,
        rule: FillRule,
        paint: Paint,
        clip: Option<ClipId>,
        blend: BlendMode,
        compose_mode: Compose,
        mask: Option<MaskId>,
    ) -> Result<(), RenderError> {
        let mask = self.use_mask(mask)?;
        let stored = self
            .resources
            .outline(outline)
            .ok_or(RenderError::UnknownOutline { outline })?;
        let to_device = compose(transform, self.viewport);
        let resolved = self.resolve_clip(clip)?;
        let bounds = self.hulls.bounds(outline, &stored.segments, &to_device);
        // A *solid* fill's visibility follows from its outline alone, so it is decided
        // here — before the implicit group a non-Normal blend would otherwise wrap it
        // in, which is the expensive half of an off-screen blended fill. A shaded fill
        // waits until `shaded_geometry` has resolved its paint: an unknown ramp or mesh
        // id must refuse by name wherever the fill happens to land, and a refusal that
        // depended on the viewport would be a worse defect than the work it saved.
        if matches!(paint, Paint::Solid(_)) && bounds.is_none_or(|b| self.culled(b, &resolved)) {
            return Ok(());
        }
        if blend != BlendMode::Normal && self.style == DrawStyle::Over {
            return self.fill_through_blend_group(
                outline,
                transform,
                rule,
                paint,
                clip,
                blend,
                compose_mode,
                mask,
            );
        }
        // A staged operator names its own pass; anything else inherits the enclosing
        // group's style, which is knockout or over (ADR 0025 refuses the combination at
        // the builder, so these cases cannot overlap).
        let style = match compose_mode {
            Compose::Src => DrawStyle::Knockout,
            Compose::DestOut => DrawStyle::DestOut,
            Compose::Plus => DrawStyle::Plus,
            Compose::SrcOver => self.style,
        };
        self.distinct_outlines.insert(outline.0);
        self.segments = self.segments.saturating_add(stored.segments.len() as u64);
        let rule = match rule {
            FillRule::NonZero => Rule::NonZero,
            FillRule::EvenOdd => Rule::EvenOdd,
        };
        if let Paint::Solid(color) = paint {
            let Some(bounds) = bounds else {
                return Ok(()); // no geometry: draws nothing
            };
            // A rectangle is not a path (RENDER_LIBRARY.md §6.4) and a fill is the only
            // way a document says one: the caller's 995-page corpus emits no
            // `Command::Rect` at all, so this is the door real pages take to ADR 0007's
            // lane (ADR 0047). The three conditions are the shaded arm's below, and each
            // is the same requirement seen on a different paint:
            //
            // - a **residue** clip has to multiply into a coverage mask, and the
            //   analytic lane has nowhere to put one;
            // - an **oblique** transform makes the four edges a parallelogram, whose
            //   coverage `rect.wgsl` cannot express;
            // - `rect_hint` is what says the outline is four axis-aligned edges at all.
            //
            // The **fill rule** is deliberately not among them. `axis_aligned_rect`
            // accepts exactly one closed subpath of four corners, and for a simple
            // closed curve §8.5.3.3.3's even-odd rule and §8.5.3.3.2's non-zero rule
            // bound the same region: a ray from an interior point crosses the boundary
            // an odd number of times, and those crossings sum to a winding of ±1
            // whichever direction the corners were given in. So `rule` cannot change
            // the mark, which is why it is not asked about here.
            if resolved.residues.is_none()
                && transform_preserves_axes(&to_device)
                && let Some(hint) = stored.rect_hint
            {
                match self.clipped_device_rect(hint, &to_device, &resolved) {
                    Some(device_rect) => {
                        self.push_rect_instance(device_rect, color, style, mask)?;
                    }
                    // The cull above tests bounds inflated by `CULL_MARGIN`; this one
                    // is exact, so it can still fire, and a command that reaches no
                    // pixel is counted wherever it is discovered.
                    None => self.note_culled(),
                }
                return Ok(());
            }
            let placement = SolidFill {
                outline,
                transform,
                to_device,
                rule,
                color,
                bounds,
                style,
                mask,
            };
            return self.fill_solid(&placement, &resolved);
        }
        // Shading, mesh or function paint (§8.7.4.5): one quad over a coverage source.
        // The rect-hinted case needs no scratch tile — analytic coverage, mirroring the
        // rectangle lane (ADR 0011).
        let Some(rare) = self.rare_paint(paint)? else {
            return Ok(());
        };
        // The paint is resolved, so the fill may now be dropped for being out of
        // sight — the half of the solid lane's test that had to wait.
        if bounds.is_none_or(|b| self.culled(b, &resolved)) {
            return Ok(());
        }
        if resolved.residues.is_none()
            && transform_preserves_axes(&to_device)
            && let Some(rect) = stored.rect_hint
        {
            return self.push_rare_rect(rare, rect, &to_device, &resolved, style, mask);
        }
        let span = self.clock.start();
        let polylines = raster::flatten(&stored.segments, to_device);
        self.clock.geometry(span);
        self.push_rare_coverage(rare, &polylines, rule, &resolved, style, mask)
    }

    /// The solid arm of the fill walk: three lanes, and the cache decides between them.
    ///
    /// In order of preference, and each condition is stated where it is asked:
    /// [`Encoder::take_gpu_lane`] for a tile the cache is no use for, the glyph lane for
    /// one it will hold and re-read, the scratch path for everything left — a residue
    /// clip, or a tile too large for the atlas whose triangles cost more than its
    /// coverage.
    fn fill_solid(&mut self, fill: &SolidFill, resolved: &ResolvedClip) -> Result<(), RenderError> {
        let stored = self
            .resources
            .outline(fill.outline)
            .ok_or(RenderError::UnknownOutline {
                outline: fill.outline,
            })?;
        let (bx0, by0, bx1, by1) = fill.bounds;
        let (tile_width, tile_height) = (tile_side(bx0, bx1), tile_side(by0, by1));
        // What the cache would do with this placement, asked once and answered by the
        // atlas and the census together (ADR 0029). Both lanes below read this one
        // answer: a lane chosen on one reading of the cache and taken on another is how
        // a tile ends up rasterised twice, or not at all.
        let placed_once =
            self.census
                .placed_once(fill.outline.0, linear_bits(fill.transform), fill.rule);
        let cache = self.prospect_for(
            GlyphPlacement::of(fill.outline, &fill.to_device, fill.rule, self.quantum),
            tile_width,
            tile_height,
            placed_once,
        )?;
        // The GPU lane takes the outline as it was uploaded — quadratics, not polylines
        // — which is the whole of why its cost does not grow with the magnification:
        // there is no flattening here to be done again at a new scale, and no atlas in
        // front of it to be cold (ADR 0016).
        if !stored.quads.is_empty()
            && self.take_gpu_lane(
                resolved,
                cache,
                tile_width,
                tile_height,
                stored.quads.triangle_count(),
            )
        {
            let Some(tile) = self.visible_tile(fill.bounds, resolved) else {
                return Ok(());
            };
            let quads = &stored.quads;
            let device = fill.to_device;
            return self.push_gpu_tile(
                tile,
                fill.rule,
                fill.color,
                resolved,
                fill.style,
                fill.mask,
                |out, origin, clip| {
                    quads.append_triangles(
                        |p| {
                            let q = apply(&device, p);
                            [q.x + origin[0], q.y + origin[1]]
                        },
                        clip,
                        out,
                    );
                },
            );
        }
        // Cacheable is a question for the atlas — how much of it this tile would take —
        // rather than a constant here (ADR 0024). A residue chain still takes the
        // scratch path: the clip multiplies into the tile, so the tile is not the glyph
        // and would poison the cache for every other placement of it.
        if let (None, Some((placement, entry))) = (resolved.residues.as_ref(), cache.admission()) {
            // The tile is rasterised at the *quantised* phase and drawn at the integer
            // origin: that split is the whole of what the quantum does (ADR 0009).
            let tile_transform = DeviceTransform {
                e: placement.phase[0],
                f: placement.phase[1],
                ..fill.to_device
            };
            return self.enqueue(Job::glyph(
                &stored.segments,
                tile_transform,
                fill.rule,
                placement.key,
                placement.origin,
                entry,
                // The tile the atlas was just asked about, which is the hull's box and so
                // an upper bound on the one the rasteriser will make.
                u64::from(tile_width).saturating_mul(u64::from(tile_height)),
                Draw::new(fill.color, resolved.rect, fill.style, fill.mask),
            ));
        }
        if let Some(rect) = self.deferrable_bounds(resolved) {
            let bound = self.tile_bound(fill.bounds, resolved);
            return self.enqueue(Job::sheet(
                &stored.segments,
                fill.to_device,
                None,
                fill.rule,
                rect,
                bound,
                Draw::new(fill.color, resolved.rect, fill.style, fill.mask),
            ));
        }
        let span = self.clock.start();
        let polylines = raster::flatten(&stored.segments, fill.to_device);
        self.clock.geometry(span);
        self.push_coverage_styled(
            &polylines, fill.rule, fill.color, resolved, fill.style, fill.mask,
        )
    }

    /// §11.3.5 for a single element: the implicit one-element group a blended fill
    /// draws through, so the blend function sees the element's own colour rather than
    /// the accumulated layer's.
    ///
    /// Inside a knockout group the element composites with the transparent initial
    /// backdrop, where every blend mode degenerates to Normal — §11.4.6 with §11.3.6's
    /// αb = 0 — so knockout draws never come here.
    #[allow(clippy::too_many_arguments)] // the fill's own parameters, forwarded once
    fn fill_through_blend_group(
        &mut self,
        outline: OutlineId,
        transform: Affine,
        rule: FillRule,
        paint: Paint,
        clip: Option<ClipId>,
        blend: BlendMode,
        compose_mode: Compose,
        mask: Option<u32>,
    ) -> Result<(), RenderError> {
        let child = self.plan_child(|encoder| {
            encoder.encode_fill(
                outline,
                transform,
                rule,
                paint,
                clip,
                BlendMode::Normal,
                compose_mode,
                None,
            )
        })?;
        self.push_op(Op::Child(ChildOp::implicit_blend_group(child, blend, mask)))
    }

    fn resolve_clip(&mut self, clip: Option<ClipId>) -> Result<ResolvedClip, RenderError> {
        match clip {
            Some(id) => self
                .clips
                .resolve(id, self.scene, self.viewport, self.resources),
            None => Ok(open_clip()),
        }
    }

    /// Charge one coverage tile, remembering how much of the sheet has been paid for
    /// tile by tile — the sheet's own extent is charged once at the end (ADR 0021), and
    /// this is what keeps that from charging twice for the same bytes.
    fn charge_tile(&mut self, width: u32, height: u32) -> Result<(), RenderError> {
        let bytes = u64::from(width).saturating_mul(u64::from(height));
        self.scratch_charged = self.scratch_charged.saturating_add(bytes);
        self.charge(bytes)
    }

    /// Spend against the frame's budget, refusing before anything is allocated.
    ///
    /// The queue drains first, because a refusal names the running total and a queued
    /// mark has not added its own yet: charging out of encounter order would refuse the
    /// same frames with a different number in the message (`parallel`).
    fn charge(&mut self, bytes: u64) -> Result<(), RenderError> {
        self.drain_queue()?;
        let needed = self.spent.saturating_add(bytes);
        if needed > self.budget {
            return Err(RenderError::FrameBudgetExceeded {
                needed,
                budget: self.budget,
            });
        }
        self.spent = needed;
        Ok(())
    }
}
