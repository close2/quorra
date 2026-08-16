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
//! # This file, and the sixteen modules under it
//!
//! What is left in *this* file is the walk itself: the encoder's working state, the one
//! pass over the scene's commands, the dispatch that sends each to its arm, and the two
//! things every arm goes through — the clip resolver and the frame budget.
//!
//! Each part is private, which is ADR 0051's rule read for a module that is already
//! private to the crate: nothing outside `encode` can name one, so this layout stays
//! ours to change. The names below are not links for the same reason — a private module
//! is not in the published documentation, and this list is the only place the structure
//! survives into it.
//!
//! **One arm per command**, in the order the dispatch takes them:
//!
//! - `rect` — ADR 0007's analytic lane, and the device rectangle both commands that
//!   reach it are held to.
//! - `fill` — which of three lanes draws a fill, and the description all three are
//!   handed.
//! - `stroke` — a width §4.5 resolved before it reached us, expanded into a fill, plus
//!   the reach that expansion adds to every visibility test.
//! - `rare` — the image and shading lanes: the quads the brief's §0 calls the rare case
//!   (ADR 0011).
//! - `layer` — a child layer: the group that becomes one, and the composite that puts
//!   it back (ISO 32000-2 §11.4.5).
//!
//! **What an arm draws through:**
//!
//! - `device_space` — where a scene coordinate lands, and whether it lands on the
//!   target at all.
//! - `clips` — a chain of clip ids as a rectangle and a residue (ADR 0007, ADR 0030).
//! - `residue` — a chain's residue region, kept so that it is rasterised once rather
//!   than once per mark (ADR 0049).
//! - `coverage` — where a mark's coverage comes from, and the conditions that choose
//!   between the two ways of making one.
//! - `scratch` — the frame's coverage sheet, and the shelf packing that fills it
//!   (ADR 0021, ADR 0034).
//! - `hull` — the memo that bounds a placement by translating its neighbour's box
//!   rather than transforming the outline again (ADR 0045).
//! - `parallel` — the seam that lets a run of marks rasterise off this thread when the
//!   host asked for that (ADR 0054).
//! - `function` — the quad a §7.10.5 program paints, and the reports an empty operand
//!   stack owes (ADR 0053).
//!
//! **What the walk writes into, and what it hands back:**
//!
//! - `instance` — one mark's instance bytes, and the run of consecutive marks it joins.
//! - `plan` — the ops one layer draws, in order, and the box that has to hold them.
//! - `encoded` — what one walk hands back, and what the walk cost.

mod clips;
mod coverage;
mod device_space;
mod encoded;
mod fill;
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
mod stroke;

#[cfg(test)]
mod tests;

use std::collections::HashSet;

use quorra_scene::{ClipId, Command, Rect, Scene};

use clips::{ClipResolver, ResolvedClip, open_clip};
use device_space::target_rect;
use encoded::finish;
use instance::instance_reserve;
use parallel::Job;

use crate::atlas::{AtlasStore, GlyphKey};
use crate::census::Census;
use crate::error::RenderError;
use crate::instrument::EncodeClock;
use crate::keyhash::FastSet;
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
