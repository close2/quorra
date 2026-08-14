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
//! rather than transforming the outline again (ADR 0045). What is left here is the walk
//! itself and the two lanes it exists for.

mod clips;
mod hull;
mod rare;
mod residue;
mod scratch;

use std::collections::HashSet;

use quorra_scene::{
    Affine, BlendMode, ClipId, Command, Compose, FillRule, MaskId, MaskKind, OutlineId, Paint,
    Point, Rect, Scene,
};

use clips::{ClipResolver, OPEN_CLIP, ResolvedClip, open_clip};

use crate::atlas::{AtlasEntry, AtlasStore, CacheProspect, GlyphKey, GlyphPlacement};
use crate::census::Census;
use crate::error::RenderError;
use crate::instrument::EncodeClock;
use crate::keyhash::FastSet;
use crate::raster::{self, DeviceTransform, Polyline, Rule};
use crate::resources::ResourceStore;
use crate::startup::Coverage;
use crate::viewport::Viewport;

pub(crate) use rare::{ImageOp, PaintSource, ShadedOp};
use residue::ResidueRegions;
pub(crate) use scratch::Scratch;
use scratch::ScratchPacker;

/// Bytes per rectangle instance: device rect (4 × f32), premultiplied colour
/// (4 × f32). Must match `rect.wgsl`.
pub(crate) const RECT_INSTANCE_STRIDE: u64 = 32;

/// Bytes per coverage-quad instance: dest min (2), size (2), texel origin + source
/// selector (4), premultiplied colour (4), clip rect (4) — 16 × f32. Must match
/// `coverage.wgsl`.
pub(crate) const QUAD_INSTANCE_STRIDE: u64 = 64;

/// A shape's device extent along one axis, as the tile that would hold it: the same
/// `floor`/`ceil` the rasteriser uses, so the number the atlas is asked about is the
/// number of texels it would be given.
fn tile_side(low: f32, high: f32) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped below
    {
        (high.ceil() - low.floor())
            .max(0.0)
            .min(f32::from(u16::MAX)) as u32
    }
}

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

/// Which lane a batch draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BatchKind {
    Rect,
    Quad,
}

/// How a batch composites: ordinary premultiplied over, or the knockout two-pass
/// (per-element erase by shape, then additive deposit — ADR 0010, §11.4.6/§4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DrawStyle {
    Over,
    Knockout,
    /// §11.4.6's first stage alone: scale the backdrop by `1 − shape` and deposit
    /// nothing ([`Compose::DestOut`], ADR 0025). The same erase pass the knockout pair
    /// opens with, asked for by name.
    DestOut,
    /// §11.4.6's second stage alone: add the mark, premultiplied ([`Compose::Plus`]).
    Plus,
}

/// A run of consecutive instances in one lane with one style and one soft mask, in
/// scene order — the painter's algorithm survives switching by batch breaks, not by
/// reordering.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Batch {
    pub kind: BatchKind,
    pub first: u32,
    pub count: u32,
    pub style: DrawStyle,
    /// The soft mask sampled by these draws, as a mask index.
    pub mask: Option<u32>,
}

/// One node of the frame's layer tree: what to draw, in order, including child
/// layers to composite at their place in the order.
#[derive(Debug, Default)]
pub(crate) struct LayerPlan {
    pub ops: Vec<Op>,
    /// Device-space union of everything this plan draws, including its children after
    /// their own clips — `None` for a plan that marks nothing (ADR 0036).
    ///
    /// What the compositor allocates for it, rather than the whole target: on the three
    /// corpus pages that refuse for bytes at 4× the plans cover 0.0 %, 0.1–0.4 % and
    /// 4–6.5 % of the page. The root is the exception and is always the target's size,
    /// because it *is* the target.
    pub bounds: Option<[f32; 4]>,
}

impl LayerPlan {
    /// Grow the plan's bounds to hold `rect`, which is already in device space.
    fn mark(&mut self, rect: [f32; 4]) {
        if !(rect[2] > rect[0] && rect[3] > rect[1]) {
            return; // a mark with no area moves nothing
        }
        self.bounds = Some(match self.bounds {
            None => rect,
            Some(b) => [
                b[0].min(rect[0]),
                b[1].min(rect[1]),
                b[2].max(rect[2]),
                b[3].max(rect[3]),
            ],
        });
    }
}

#[derive(Debug)]
pub(crate) enum Op {
    Draw(Batch),
    /// One image quad (boxed: rare on real pages, and `Op` stays small for the
    /// common draws).
    Image(Box<ImageOp>),
    /// One shading or mesh quad.
    Shaded(Box<ShadedOp>),
    Child(ChildOp),
}

/// Composite one finished child layer onto this layer (§11.4.5), exactly once.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ChildOp {
    /// Index into `Encoded::layers`.
    pub layer: usize,
    /// §11.3.5 mode, in `BlendMode`'s declaration order (the shader's numbering).
    pub mode: u32,
    /// The group's constant alpha.
    pub alpha: f32,
    /// The group's resolved clip rectangle, device space.
    pub clip_rect: [f32; 4],
    /// Clip-residue placement in the scratch image: region rect then texel origin;
    /// an empty region means no residue.
    pub residue_rect: [f32; 4],
    pub residue_origin: [f32; 2],
    /// §11.4.6's stage this group *is*, when it is one: 0 ordinary, 1 erase by the
    /// group's own alpha, 2 add it (ADR 0033).
    pub compose: u32,
    /// The group's soft mask, as a mask index.
    pub mask: Option<u32>,
    /// §11.4.5's isolated group (the ordinary case) or §11.4.4's non-isolated one,
    /// whose layer is seeded with the backdrop and interpolated back onto it
    /// (ADR 0019). The implicit one-element groups §11.3.5 needs for a blended
    /// element are isolated: the wrapper is a device trick, not a PDF group.
    pub isolated: bool,
}

impl ChildOp {
    /// The composite of the implicit one-element group a blended element draws through
    /// (ISO 32000-2 §11.3.5).
    ///
    /// §11.3.5's blend function takes the *group's* backdrop, so an element whose blend
    /// mode is not `Normal` is wrapped in a group holding it alone and composited once,
    /// under the element's own mode — which is what makes the mode see the element's
    /// colour rather than the accumulated layer's.
    ///
    /// The wrapper is a device trick and not a PDF group, and every field this fixes
    /// says so: no group alpha, because the element's paint carries its own; no clip
    /// rectangle and no residue, because the element resolved its clip before it was
    /// wrapped and draws through it inside the layer; no stage of §11.4.6 (ADR 0033's
    /// `compose`), because a group that *is* an erase or a deposit is a group the scene
    /// asked for; and isolated, because a wrapper has no backdrop of its own to seed
    /// (ADR 0019). The soft mask is the one thing the wrapper does take over: the
    /// element is re-encoded without it, so the mask weighs the finished group once
    /// rather than each draw inside it.
    ///
    /// Three arms reach this — a fill, a stroke and an image — and while each wrote the
    /// eleven fields out, a field that came to differ between them would have been
    /// three lanes disagreeing about one clause.
    fn implicit_blend_group(layer: usize, blend: BlendMode, mask: Option<u32>) -> Self {
        Self {
            layer,
            mode: blend_word(blend),
            alpha: 1.0,
            clip_rect: OPEN_CLIP,
            residue_rect: [0.0; 4],
            residue_origin: [0.0; 2],
            compose: 0,
            mask,
            isolated: true,
        }
    }
}

/// A soft mask's realisation plan: its group's layer tree plus the reduction
/// parameters (§11.5, mirrored byte-for-byte against the caller's rule).
#[derive(Debug)]
pub(crate) struct MaskPlan {
    /// Index into `Encoded::layers` of the mask group's plan.
    pub root: usize,
    /// 0 = Alpha (§11.5.2), 1 = Luminosity (§11.5.3).
    pub kind_word: u32,
    /// The luminosity backdrop, device RGB (unused for Alpha).
    pub backdrop: [f32; 3],
    /// §11.6.5.1's transfer table, identity when the scene gave none.
    pub table: [u8; 256],
}

/// The encoded frame.
#[derive(Debug)]
pub(crate) struct Encoded {
    pub rect_instances: Vec<u8>,
    pub quad_instances: Vec<u8>,
    /// The page's own plan; child layers live in `layers`.
    pub root: LayerPlan,
    pub layers: Vec<LayerPlan>,
    /// Realisation plans for the masks this frame uses, indexed by `MaskId`.
    pub mask_plans: Vec<Option<MaskPlan>>,
    pub scratch: Option<Scratch>,
    /// The raw ids of every image, ramp and mesh this frame draws, for the device
    /// to realise as textures before the passes run.
    pub used_images: Vec<u32>,
    pub used_ramps: Vec<u32>,
    pub used_meshes: Vec<u32>,
    pub commands: u32,
    /// Coverage tiles this frame placed on the scratch sheet, both lanes.
    pub tiles: u32,
    pub clip_distinct_regions: u32,
    /// Residue clip regions this frame rasterised once and cut every tile from, and
    /// residue rasterisations a single command's tile paid for (ADR 0049).
    pub clip_residue_regions: u32,
    pub clip_residue_tiles: u32,
    pub distinct_outlines: u32,
    pub atlas_distinct_keys: u32,
    pub segments: u32,
    /// Commands the walk rejected for reaching no pixel of the target.
    pub commands_culled: u32,
    /// Child layers the walk built and then did not composite (ADR 0041).
    pub layers_culled: u32,
    /// The GPU lane's triangles and tiles for this frame; empty under `Coverage::Cpu`
    /// and for every command that took the CPU lane anyway.
    pub winding: crate::winding::Sheet,
    /// Set when a glyph tile no longer fit the atlas and fell through to scratch.
    pub atlas_pressure: bool,
    /// Atlas bytes this frame's *distinct* keys asked for, whether they hit or missed.
    /// A repack only helps when this fits the atlas; when it does not, the frame's
    /// working set is simply larger than the cache and resetting would throw away the
    /// part that does fit and hit (ADR 0024).
    pub atlas_requested_bytes: u64,
    /// Atlas entries this frame's distinct keys reached — a resident hit or a fresh
    /// insert, counted once per key.
    ///
    /// The atlas holds at least this many entries afterwards, because insertion never
    /// removes one; anything above it belongs to an **earlier** frame. That difference
    /// is the whole of what a repack can reclaim, and ADR 0050 is the argument that when
    /// it is zero a repack provably cannot change the outcome.
    pub atlas_entries_used: u32,
    /// What the walk above spent its time on, when the caller asked for the
    /// subdivision (ADR 0023); empty otherwise.
    pub encode_phases: EncodeClock,
}

impl Encoded {
    /// Heap bytes this encode holds — what a [`RetainedScene`] costs to keep alive
    /// (ADR 0048).
    ///
    /// Every buffer, so the number can be budgeted against rather than sampled: the two
    /// instance streams, the coverage sheet, the GPU lane's vertices and tiles, the plan
    /// tree's op lists (with the boxed image and shading operands ADR 0011's rare lanes
    /// allocate), the mask plans and the three resource-id lists. Vector *capacity* is
    /// not read — a `Vec`'s spare capacity is not something this frame decided — so the
    /// number is what the encode filled, which is also what a second one would fill.
    ///
    /// Called once per stored encode, never per frame.
    pub(crate) fn retained_bytes(&self) -> u64 {
        let bytes_of =
            |count: usize, each: usize| -> u64 { (count as u64).saturating_mul(each as u64) };
        let plan_bytes = |plan: &LayerPlan| -> u64 {
            plan.ops.iter().fold(
                bytes_of(plan.ops.len(), size_of::<Op>()),
                |total, op| match op {
                    Op::Image(_) => total.saturating_add(size_of::<ImageOp>() as u64),
                    Op::Shaded(_) => total.saturating_add(size_of::<ShadedOp>() as u64),
                    Op::Draw(_) | Op::Child(_) => total,
                },
            )
        };
        let scratch = self
            .scratch
            .as_ref()
            .map_or(0, |scratch| scratch.data.len() as u64);
        [
            self.rect_instances.len() as u64,
            self.quad_instances.len() as u64,
            scratch,
            bytes_of(self.winding.vertices.len(), size_of::<f32>()),
            bytes_of(self.winding.tiles.len(), size_of::<crate::pane::Tile>()),
            plan_bytes(&self.root),
            self.layers.iter().map(plan_bytes).sum::<u64>(),
            bytes_of(self.layers.len(), size_of::<LayerPlan>()),
            bytes_of(self.mask_plans.len(), size_of::<Option<MaskPlan>>()),
            bytes_of(self.used_images.len(), size_of::<u32>()),
            bytes_of(self.used_ramps.len(), size_of::<u32>()),
            bytes_of(self.used_meshes.len(), size_of::<u32>()),
        ]
        .into_iter()
        .fold(0_u64, u64::saturating_add)
    }
}

fn compose(transform: Affine, viewport: &Viewport<'_>) -> DeviceTransform {
    let t = transform.then(viewport.transform);
    DeviceTransform {
        a: t.a,
        b: t.b,
        c: t.c,
        d: t.d,
        e: t.e,
        f: t.f,
    }
}

fn transform_preserves_axes(t: &DeviceTransform) -> bool {
    // Exact zeros, as in `Affine::preserves_axes`: document transforms carry them.
    #[allow(clippy::float_cmp)]
    {
        (t.b == 0.0 && t.c == 0.0) || (t.a == 0.0 && t.d == 0.0)
    }
}

fn apply(t: &DeviceTransform, p: Point) -> Point {
    Point::new(t.a * p.x + t.c * p.y + t.e, t.b * p.x + t.d * p.y + t.f)
}

/// How far a lane may mark outside the device bounds a cull is tested against.
///
/// Two device pixels, and each one is a real mechanism rather than a safety margin:
/// the glyph lane rasterises at a *quantised* sub-pixel phase, which moves a tile by
/// under one pixel from the transform its bounds were taken from
/// ([`Encoder::push_glyph`]); and every coverage tile expands to whole pixels by
/// `floor`/`ceil`, which reaches under one pixel further again. Flattening adds
/// nothing to this — a flattened point lies on the curve, which lies inside the
/// control hull [`hull::HullMemo::bounds`] measures.
const CULL_MARGIN: f32 = 2.0;

/// The target's own pixel rectangle — what a command has to reach to draw anything.
///
/// Its corners are integers, and that is load-bearing rather than incidental: an
/// edge landing exactly on a pixel boundary contributes full coverage to the pixel
/// inside it and none to the pixel outside, so a rectangle *clipped* to this
/// rectangle covers every pixel it covered before. That is why the analytic lane may
/// intersect its geometry with the target and not merely test against it, while a
/// clip at a fractional coordinate — a real edge, which must antialias — is the
/// intersection ADR 0007 already reasons about.
#[allow(clippy::cast_precision_loss)] // viewport extents are far below f32's exact range
fn target_rect(viewport: &Viewport<'_>) -> Rect {
    Rect::new(
        Point::new(0.0, 0.0),
        Point::new(viewport.width as f32, viewport.height as f32),
    )
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
    /// One control-hull box per `(outline, linear part)`, so the 3 502 repeats of a
    /// dense page's 818 letterforms add a translation instead of transforming 37 control
    /// points again — 21 % of the encode, and [`hull`] carries the benchmark and the
    /// argument that it changes no bit of any box.
    hulls: hull::HullMemo,
}

/// Bytes to reserve for one instance stream, from the command count the frame budget
/// was checked against.
///
/// Called only after the budget check, so the value is bounded by the caller's stated
/// `frame_budget_bytes`: this reserves what the frame has already been *charged*, never
/// more. The arithmetic is `u64` because the check is, and a product that does not fit
/// a `usize` reserves **nothing** rather than saturating — a `Vec` that grows is
/// correct and slower, while `with_capacity(usize::MAX)` is an abort.
fn instance_reserve(commands: usize, stride: u64) -> usize {
    usize::try_from((commands as u64).saturating_mul(stride)).unwrap_or(0)
}

/// §6.4's instrument: how many **distinct clip regions** this frame resolved, keyed by
/// the region itself and never by an identifier.
///
/// The caller's clip-mask cache once answered all 303 lookups a page made and built 303
/// identical page-wide masks because its key was a name; the same page collapses to 1
/// here. The bits of the four floats are the key, because that is what "the same
/// rectangle" means without an `Eq` on `f32`.
fn distinct_clip_regions(clips: &ClipResolver) -> u32 {
    let mut distinct = HashSet::new();
    for resolved in clips.resolved.iter().flatten() {
        distinct.insert([
            resolved.rect.min.x.to_bits(),
            resolved.rect.min.y.to_bits(),
            resolved.rect.max.x.to_bits(),
            resolved.rect.max.y.to_bits(),
        ]);
    }
    u32::try_from(distinct.len()).unwrap_or(u32::MAX)
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
        segments: 0,
        culled: 0,
        culled_layers: 0,
        atlas_pressure: false,
        atlas_requested_bytes: 0,
        atlas_entries_used: 0,
        scratch_charged: 0,
        clock: EncodeClock::new(instrument),
        hulls: hull::HullMemo::default(),
    };

    for (index, command) in commands.iter().enumerate() {
        encoder.command(index, command)?;
    }

    // The sheet's extent is only known once every tile has been placed, so the GPU
    // lane learns it here rather than carrying a guess: its triangles are already in
    // sheet coordinates, and what was missing was how large the sheet turned out to
    // be. Then the lane's own cost is charged — scene-derived arithmetic, priced where
    // nothing has been allocated yet, against the same one number (principle 3). A
    // frame whose sheet holds no GPU tiles is charged nothing here, because it
    // allocates nothing there: `Sheet::device_bytes` states that condition once.
    let mut winding = std::mem::take(&mut encoder.winding);
    let packer = std::mem::replace(&mut encoder.scratch, ScratchPacker::new(1, 1));
    let tiles = packer.placed;
    let scratch = packer.finish();
    if let Some(sheet) = scratch.as_ref() {
        winding.width = sheet.width;
        winding.height = sheet.height;
        // The sheet is one texture, and until ADR 0021 the only thing charged for it
        // was the area of the tiles *on* it — so the largest scene-derived allocation
        // a page of path work makes was the one number nobody counted, which is the
        // reverse of what principle 3 asks. Shelf packing leaves gaps, and the gaps
        // are allocated too: charge the difference, once, now that the extent is known.
        let sheet_bytes = u64::from(sheet.width).saturating_mul(u64::from(sheet.height));
        encoder.charge(sheet_bytes.saturating_sub(encoder.scratch_charged))?;
    }
    encoder.charge(winding.device_bytes())?;

    let sorted = |set: HashSet<u32>| {
        let mut ids: Vec<u32> = set.into_iter().collect();
        ids.sort_unstable();
        ids
    };

    Ok(Encoded {
        rect_instances: encoder.rect_instances,
        quad_instances: encoder.quad_instances,
        root: encoder.root,
        layers: encoder.layers,
        mask_plans: encoder.mask_plans,
        scratch,
        used_images: sorted(encoder.used_images),
        used_ramps: sorted(encoder.used_ramps),
        used_meshes: sorted(encoder.used_meshes),
        encode_phases: encoder.clock,
        tiles,
        commands: u32::try_from(commands.len()).unwrap_or(u32::MAX),
        clip_distinct_regions: distinct_clip_regions(&encoder.clips),
        clip_residue_regions: encoder.residue.regions,
        clip_residue_tiles: encoder.residue.tiles,
        distinct_outlines: u32::try_from(encoder.distinct_outlines.len()).unwrap_or(u32::MAX),
        atlas_distinct_keys: u32::try_from(encoder.atlas_keys.len()).unwrap_or(u32::MAX),
        segments: u32::try_from(encoder.segments).unwrap_or(u32::MAX),
        commands_culled: encoder.culled,
        layers_culled: encoder.culled_layers,
        winding,
        atlas_pressure: encoder.atlas_pressure,
        atlas_requested_bytes: encoder.atlas_requested_bytes,
        atlas_entries_used: encoder.atlas_entries_used,
    })
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
            Command::Group { spec, commands } => {
                let mask = self.use_mask(spec.mask)?;
                let resolved = self.resolve_clip(spec.clip)?;
                let outer_style = self.style;
                let child = self.plan_child(|encoder| {
                    // §11.4.6 binds inside this group. What the elements draw *onto* is
                    // `spec.isolated`: transparent for §11.4.5's group, a copy of the
                    // backdrop for §11.4.4's — a decision the compositor makes when it
                    // seeds the layer, not one the elements can see.
                    encoder.style = if spec.knockout {
                        DrawStyle::Knockout
                    } else {
                        DrawStyle::Over
                    };
                    for (i, command) in commands.iter().enumerate() {
                        encoder.command(i, command)?;
                    }
                    Ok(())
                });
                self.style = outer_style;
                let child = child?;
                let (residue_rect, residue_origin) = self.plan_group_residue(&resolved)?;
                self.push_op(Op::Child(ChildOp {
                    layer: child,
                    mode: blend_word(spec.blend),
                    alpha: spec.alpha,
                    clip_rect: [
                        resolved.rect.min.x,
                        resolved.rect.min.y,
                        resolved.rect.max.x,
                        resolved.rect.max.y,
                    ],
                    residue_rect,
                    residue_origin,
                    compose: match spec.compose {
                        Compose::DestOut => 1,
                        Compose::Plus => 2,
                        // §11.4.6's other two are the group's own model rather than a
                        // stage of it: `SrcOver` is the ordinary composite and `Src` is
                        // what `knockout` states, which the builder refuses on a group.
                        Compose::SrcOver | Compose::Src => 0,
                    },
                    mask,
                    isolated: spec.isolated,
                }));
                Ok(())
            }
        }
    }

    /// Whether a command with these device bounds can mark no pixel, and so need
    /// never be built.
    ///
    /// Every lane already draws into `bounds ∩ clip ∩ target` and no further —
    /// `coverage_tile`, `encode_rect` and `encode_image` each intersect exactly those
    /// three. Testing it *before* the geometry exists is what makes a frame cost what
    /// it shows rather than what the page holds: at 20× magnification a page hands
    /// the encoder thousands of commands for a window that displays tens of them, and
    /// flattening the rest cost 9.35 ms of a 14.4 ms frame (ADR 0015).
    ///
    /// **Not §5's forbidden silence.** The test establishes that the command had no
    /// pixel to mark, so the frame is byte-for-byte the one that would have built the
    /// command and thrown it away — nothing is approximated and nothing is dropped
    /// that would have shown. [`Counters::commands_culled`] reports how often it
    /// fired, so the saving is measured rather than assumed.
    ///
    /// [`Counters::commands_culled`]: crate::frame::Counters::commands_culled
    /// **What it costs when it wins nothing**, since it runs once per command on the
    /// hottest walk there is: a page with nothing outside the target encodes 6% slower
    /// (5 933 commands, 0.76 → 0.81 ms; ADR 0015's table). Writing the same test on
    /// scalars instead of through [`Rect::intersection`] measured the same 6%, so the
    /// clear construction is the one that stays.
    fn culled(&mut self, bounds: (f32, f32, f32, f32), clip: &ResolvedClip) -> bool {
        let (x0, y0, x1, y1) = bounds;
        let reach = Rect::new(
            Point::new(x0 - CULL_MARGIN, y0 - CULL_MARGIN),
            Point::new(x1 + CULL_MARGIN, y1 + CULL_MARGIN),
        );
        if reach
            .intersection(clip.rect)
            .intersection(self.visible)
            .is_empty()
        {
            self.note_culled();
            return true;
        }
        false
    }

    /// Record a command that reaches no pixel of the target.
    ///
    /// Its own method because two lanes decide visibility differently and both must
    /// count: the coverage lanes test bounds inflated by [`CULL_MARGIN`], while the
    /// analytic rectangle and image lanes intersect exactly the region they draw and
    /// so need no margin at all.
    fn note_culled(&mut self) {
        self.culled = self.culled.saturating_add(1);
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
            self.push_op(Op::Child(ChildOp::implicit_blend_group(child, blend, mask)));
            return Ok(());
        }
        self.distinct_outlines.insert(outline.0);
        self.segments = self.segments.saturating_add(stored.segments.len() as u64);
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
            Paint::Shading { .. } | Paint::Mesh(_) => {
                let Some(geometry) = self.shaded_geometry(paint)? else {
                    return Ok(());
                };
                let style = self.style;
                self.push_shaded_coverage(geometry, &stroked, Rule::NonZero, &resolved, style, mask)
            }
        }
    }

    /// Plan a child layer: run `body` with the current plan switched to a fresh
    /// node, restoring on both paths.
    fn plan_child(
        &mut self,
        body: impl FnOnce(&mut Self) -> Result<(), RenderError>,
    ) -> Result<usize, RenderError> {
        let child = self.layers.len();
        self.layers.push(LayerPlan::default());
        let outer = self.current_plan;
        self.current_plan = child;
        let result = body(self);
        self.current_plan = outer;
        result?;
        Ok(child)
    }

    /// Realise a referenced soft mask's plan on first use; masks reference only
    /// earlier masks (the builder enforced it), so this terminates.
    fn use_mask(&mut self, mask: Option<MaskId>) -> Result<Option<u32>, RenderError> {
        let Some(id) = mask else { return Ok(None) };
        let index = id.0 as usize;
        if self.mask_plans[index].is_none() {
            let def = &self.scene.masks()[index];
            let commands = def.commands.clone();
            let (kind_word, backdrop) = match def.kind {
                MaskKind::Alpha => (0, [0.0, 0.0, 0.0]),
                MaskKind::Luminosity { backdrop } => (1, [backdrop.r, backdrop.g, backdrop.b]),
            };
            let table = def
                .transfer
                .as_ref()
                .map_or_else(|| quorra_scene::Transfer::identity().0, |t| t.0);
            let outer_style = self.style;
            let root = self.plan_child(|encoder| {
                encoder.style = DrawStyle::Over;
                for (i, command) in commands.iter().enumerate() {
                    encoder.command(i, command)?;
                }
                Ok(())
            });
            self.style = outer_style;
            let root = root?;
            self.mask_plans[index] = Some(MaskPlan {
                root,
                kind_word,
                backdrop,
                table,
            });
        }
        Ok(Some(id.0))
    }

    /// A composited group's clip residue, rasterised over its visible region into
    /// the scratch image for the composite pass to sample.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    #[allow(clippy::arithmetic_side_effects, clippy::cast_precision_loss)]
    fn plan_group_residue(
        &mut self,
        resolved: &ResolvedClip,
    ) -> Result<([f32; 4], [f32; 2]), RenderError> {
        let Some(_) = resolved.residues else {
            return Ok(([0.0; 4], [0.0; 2]));
        };
        let vx0 = resolved.rect.min.x.max(0.0);
        let vy0 = resolved.rect.min.y.max(0.0);
        let vx1 = resolved.rect.max.x.min(self.viewport.width as f32);
        let vy1 = resolved.rect.max.y.min(self.viewport.height as f32);
        if vx0 >= vx1 || vy0 >= vy1 {
            // Clipped to nothing: an empty region, which the composite reads as
            // "admit nothing inside, nothing outside" — the group vanishes, as an
            // empty clip demands. Represent as a 1x1 zero mask.
            let zero = raster::CoverageMask {
                left: 0,
                top: 0,
                width: 1,
                height: 1,
                coverage: vec![0],
            };
            let (sx, sy) = self.pack_scratch(&zero)?;
            return Ok(([0.0, 0.0, 1.0, 1.0], [sx as f32, sy as f32]));
        }
        let left = vx0.floor() as i32;
        let top = vy0.floor() as i32;
        let width = (vx1.ceil() as i32 - left).max(1) as u32;
        let height = (vy1.ceil() as i32 - top).max(1) as u32;
        self.charge_tile(width, height)?;
        // Present by construction: the residues Option was checked above.
        let Some(mask) = self.residue_intersection(resolved, left, top, width, height)? else {
            return Ok(([0.0; 4], [0.0; 2]));
        };
        let (sx, sy) = self.pack_scratch(&mask)?;
        Ok((
            [left as f32, top as f32, vx1.ceil(), vy1.ceil()],
            [sx as f32, sy as f32],
        ))
    }

    /// The device rectangle an axis-aligned scene rectangle marks, held to its clip and
    /// to the target — or `None` when that leaves no area, which is a command that
    /// legitimately draws nothing.
    ///
    /// This is the whole of what the analytic lane does on the CPU (ADR 0007): a
    /// rectangle intersected with a rectangle is a rectangle, so a rectangular clip
    /// costs a pixel nothing and the shader still evaluates one box. Intersecting with
    /// the target costs no pixel any coverage either — [`target_rect`] has integer
    /// corners, so an edge it introduces falls exactly on a pixel boundary. Nothing here
    /// rounds outwards, so no [`CULL_MARGIN`] is needed and none is taken.
    ///
    /// Shared by the two commands that reach the lane — [`Encoder::encode_rect`] and a
    /// solid fill whose outline is a rectangle (ADR 0047) — because two arms that
    /// computed this box separately would be two arms that could come to draw different
    /// marks for the same rectangle.
    fn clipped_device_rect(
        &self,
        rect: Rect,
        to_device: &DeviceTransform,
        resolved: &ResolvedClip,
    ) -> Option<Rect> {
        let p0 = apply(to_device, rect.min);
        let p1 = apply(to_device, rect.max);
        let device_rect = Rect::new(
            Point::new(p0.x.min(p1.x), p0.y.min(p1.y)),
            Point::new(p0.x.max(p1.x), p0.y.max(p1.y)),
        )
        .intersection(resolved.rect)
        .intersection(self.visible);
        (!device_rect.is_empty()).then_some(device_rect)
    }

    /// The rectangle arm: the analytic lane when everything is axis-aligned and
    /// rectangular, the path lane otherwise (ADR 0007).
    fn encode_rect(
        &mut self,
        rect: Rect,
        transform: Affine,
        color: quorra_scene::Color,
        clip: Option<ClipId>,
        mask: Option<MaskId>,
    ) -> Result<(), RenderError> {
        let mask = self.use_mask(mask)?;
        let resolved = self.resolve_clip(clip)?;
        let to_device = compose(transform, self.viewport);
        if transform_preserves_axes(&to_device) && resolved.residues.is_none() {
            // The analytic lane: clip applied by intersection (ADR 0007).
            let Some(device_rect) = self.clipped_device_rect(rect, &to_device, &resolved) else {
                // Clipped to nothing or off the target: draws nothing, legitimately.
                self.note_culled();
                return Ok(());
            };
            self.push_rect_instance(device_rect, color, self.style, mask);
            return Ok(());
        }
        // Oblique transform or residue clip: the rectangle is a polygon and
        // takes the path lane, exactly.
        let corners = [
            apply(&to_device, rect.min),
            apply(&to_device, Point::new(rect.max.x, rect.min.y)),
            apply(&to_device, rect.max),
            apply(&to_device, Point::new(rect.min.x, rect.max.y)),
        ];
        let bounds = corners.iter().fold(
            (
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ),
            |(x0, y0, x1, y1), p| (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
        );
        if self.culled(bounds, &resolved) {
            return Ok(());
        }
        let polylines = vec![Polyline {
            points: corners.to_vec(),
            closed: true,
        }];
        self.push_coverage(&polylines, Rule::NonZero, color, &resolved, mask)
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
                    Some(device_rect) => self.push_rect_instance(device_rect, color, style, mask),
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
        // Shading or mesh paint (§8.7.4.5): one quad over a coverage source. The
        // rect-hinted case needs no scratch tile — analytic coverage, mirroring the
        // rectangle lane (ADR 0011).
        let Some(geometry) = self.shaded_geometry(paint)? else {
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
            self.push_shaded_rect(geometry, rect, &to_device, &resolved, style, mask);
            return Ok(());
        }
        let span = self.clock.start();
        let polylines = raster::flatten(&stored.segments, to_device);
        self.clock.geometry(span);
        self.push_shaded_coverage(geometry, &polylines, rule, &resolved, style, mask)
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
        let cache = self.atlas.prospect(
            GlyphPlacement::of(fill.outline, &fill.to_device, fill.rule, self.quantum),
            tile_width,
            tile_height,
            self.census
                .placed_once(fill.outline.0, linear_bits(fill.transform), fill.rule),
        );
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
            return self.push_glyph(
                fill.outline,
                &fill.to_device,
                &placement,
                entry,
                fill.rule,
                fill.color,
                resolved,
                fill.style,
                fill.mask,
            );
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
        self.push_op(Op::Child(ChildOp::implicit_blend_group(child, blend, mask)));
        Ok(())
    }

    fn resolve_clip(&mut self, clip: Option<ClipId>) -> Result<ResolvedClip, RenderError> {
        match clip {
            Some(id) => self
                .clips
                .resolve(id, self.scene, self.viewport, self.resources),
            None => Ok(open_clip()),
        }
    }

    /// The glyph lane: rasterise (or find) the tile for this key and emit its quad.
    ///
    /// `resident` is the entry [`AtlasStore::prospect`] already found for this key, and
    /// is passed in rather than looked up again: the lane was *chosen* on that reading,
    /// so drawing on a second one could only differ if something between the two had
    /// touched the atlas — which would be the tile-rasterised-twice defect `prospect`
    /// is written against, not a case to be robust to.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    #[allow(clippy::arithmetic_side_effects, clippy::cast_precision_loss)]
    #[allow(clippy::too_many_arguments)] // one draw's parameters, threaded once
    fn push_glyph(
        &mut self,
        outline: OutlineId,
        to_device: &DeviceTransform,
        placement: &GlyphPlacement,
        resident: Option<AtlasEntry>,
        rule: Rule,
        color: quorra_scene::Color,
        resolved: &ResolvedClip,
        style: DrawStyle,
        mask: Option<u32>,
    ) -> Result<(), RenderError> {
        let [ix, iy] = placement.origin;
        let [px, py] = placement.phase;
        let key = placement.key;
        let first_use = self.atlas_keys.insert(key);

        let entry = if let Some(entry) = resident {
            if first_use {
                self.atlas_requested_bytes = self
                    .atlas_requested_bytes
                    .saturating_add(u64::from(entry.width).saturating_mul(u64::from(entry.height)));
            }
            Some(entry)
        } else {
            {
                let stored = self
                    .resources
                    .outline(outline)
                    .ok_or(RenderError::UnknownOutline { outline })?;
                let tile_transform = DeviceTransform {
                    e: px,
                    f: py,
                    ..*to_device
                };
                let span = self.clock.start();
                let polylines = raster::flatten(&stored.segments, tile_transform);
                let Some((x0, y0, x1, y1)) = raster::polyline_bounds(&polylines) else {
                    return Ok(());
                };
                let left = x0.floor() as i32;
                let top = y0.floor() as i32;
                let width = (x1.ceil() as i32 - left).max(0) as u32;
                let height = (y1.ceil() as i32 - top).max(0) as u32;
                if width == 0 || height == 0 {
                    return Ok(());
                }
                self.charge_tile(width, height)?;
                let tile = raster::fill_mask(&polylines, rule, left, top, width, height);
                self.clock.geometry(span);
                if first_use {
                    self.atlas_requested_bytes = self
                        .atlas_requested_bytes
                        .saturating_add(u64::from(width).saturating_mul(u64::from(height)));
                }
                let span = self.clock.start();
                let inserted = self.atlas.insert(key, &tile);
                self.clock.staging(span);
                if inserted.is_none() {
                    // Atlas full: this tile draws uncached, and the device repacks
                    // the atlas after the frame. Same pixels either way — one
                    // rasteriser feeds both paths.
                    self.atlas_pressure = true;
                    let dest = Point::new(ix + tile.left as f32, iy + tile.top as f32);
                    return self.push_scratch_quad(&tile, dest, color, resolved.rect, style, mask);
                }
                inserted
            }
        };
        // One count per distinct key that reached an entry, however it reached it. The
        // atlas holds at least this many entries when the frame ends, and what it holds
        // *beyond* this many is an earlier frame's — which is the only thing a repack
        // reclaims, and so the only thing that can make one worth taking (ADR 0050).
        if first_use && entry.is_some() {
            self.atlas_entries_used = self.atlas_entries_used.saturating_add(1);
        }
        if let Some(entry) = entry {
            let dest = Point::new(ix + entry.tile_left as f32, iy + entry.tile_top as f32);
            let device_rect = Rect::new(
                dest,
                Point::new(dest.x + entry.width as f32, dest.y + entry.height as f32),
            );
            if device_rect.intersection(resolved.rect).is_empty() {
                return Ok(());
            }
            self.push_quad_instance(
                dest,
                entry.width as f32,
                entry.height as f32,
                entry.x as f32,
                entry.y as f32,
                0.0, // source: atlas
                color,
                resolved.rect,
                style,
                mask,
            );
        }
        Ok(())
    }

    /// The path lane: rasterise coverage for these polylines over the visible
    /// region, multiply residue clips in, pack into scratch, emit the quad.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    #[allow(clippy::arithmetic_side_effects, clippy::cast_precision_loss)]
    fn push_coverage(
        &mut self,
        polylines: &[Polyline],
        rule: Rule,
        color: quorra_scene::Color,
        resolved: &ResolvedClip,
        mask: Option<u32>,
    ) -> Result<(), RenderError> {
        let style = self.style;
        self.push_coverage_styled(polylines, rule, color, resolved, style, mask)
    }

    #[allow(clippy::too_many_arguments)] // one draw's parameters, threaded once
    #[allow(clippy::cast_precision_loss)]
    fn push_coverage_styled(
        &mut self,
        polylines: &[Polyline],
        rule: Rule,
        color: quorra_scene::Color,
        resolved: &ResolvedClip,
        style: DrawStyle,
        mask: Option<u32>,
    ) -> Result<(), RenderError> {
        // Already-flattened geometry — a stroke's expansion, an oblique rectangle — has
        // one triangle per point, since `append_triangles` fans each polyline from its
        // own start.
        let flattened_triangles: usize = polylines.iter().map(|line| line.points.len()).sum();
        // **No cache is in play here**, whatever the tile's size: this geometry is
        // already flattened — a stroke's expansion, an oblique rectangle, a fill the
        // glyph lane declined — and the atlas caches outlines by key, not polylines. So
        // the lane is decided by the triangle floor alone (ADR 0026), which is the whole
        // of the comparison when neither side can cache. Asking the atlas whether it
        // *would* admit a tile it will never be offered is what ADR 0028 did here, and
        // it kept small strokes on the CPU lane for a cache that was never an option.
        if let Some(bounds) = raster::polyline_bounds(polylines)
            && self.take_gpu_lane(
                resolved,
                CacheProspect::TooLarge,
                tile_side(bounds.0, bounds.2),
                tile_side(bounds.1, bounds.3),
                flattened_triangles,
            )
        {
            // Flattened already — a stroke was expanded on the CPU (§8.4.3) and an
            // oblique rectangle is four corners — so what moves to the device is the
            // rasterising, which is the half that costs (ADR 0015).
            let Some(tile) = self.visible_tile(bounds, resolved) else {
                return Ok(());
            };
            return self.push_gpu_tile(
                tile,
                rule,
                color,
                resolved,
                style,
                mask,
                |out, origin, clip| {
                    crate::outline::append_polyline_triangles(
                        polylines,
                        |p| [p.x + origin[0], p.y + origin[1]],
                        clip,
                        out,
                    );
                },
            );
        }
        let Some(tile) = self.coverage_tile(polylines, rule, resolved)? else {
            return Ok(());
        };
        let dest = Point::new(tile.left as f32, tile.top as f32);
        self.push_scratch_quad(&tile, dest, color, resolved.rect, style, mask)
    }

    /// Rasterise the visible coverage of these polylines — shape ∩ clip ∩ target,
    /// residue clips multiplied in — or `None` when nothing is visible.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    #[allow(clippy::arithmetic_side_effects, clippy::cast_precision_loss)]
    fn coverage_tile(
        &mut self,
        polylines: &[Polyline],
        rule: Rule,
        resolved: &ResolvedClip,
    ) -> Result<Option<raster::CoverageMask>, RenderError> {
        let Some((x0, y0, x1, y1)) = raster::polyline_bounds(polylines) else {
            return Ok(None);
        };
        // The visible region: shape ∩ clip rectangle ∩ target.
        let vx0 = x0.max(resolved.rect.min.x).max(0.0);
        let vy0 = y0.max(resolved.rect.min.y).max(0.0);
        let vx1 = x1.min(resolved.rect.max.x).min(self.viewport.width as f32);
        let vy1 = y1.min(resolved.rect.max.y).min(self.viewport.height as f32);
        if vx0 >= vx1 || vy0 >= vy1 {
            return Ok(None);
        }
        let left = vx0.floor() as i32;
        let top = vy0.floor() as i32;
        let width = (vx1.ceil() as i32 - left).max(0) as u32;
        let height = (vy1.ceil() as i32 - top).max(0) as u32;
        if width == 0 || height == 0 {
            return Ok(None);
        }
        self.charge_tile(width, height)?;
        let span = self.clock.start();
        let mut tile = raster::fill_mask(polylines, rule, left, top, width, height);
        self.clock.geometry(span);

        // The clip meets the mark here, and **this one still multiplies** — deliberately,
        // and not for the reason the chain intersects (ADR 0030). §8.5.4 asks for an
        // intersection of the object's shape with the clipping path, and *neither* `min`
        // nor a product is that: the exact answer is the area of the two regions'
        // intersection inside the pixel, which only a conflation-free rasteriser has.
        // What separates the two estimates is whether the boundaries are related, and
        // here they usually are not — where a chain's links are one region restated,
        // which is what makes `min` exact for them and only an upper bound here.
        // Measured, and it is the reason this is a choice rather than a conclusion:
        // moving this site to `min` as well moves no page of the caller's corpus, in
        // either direction, and no page's printed numbers.
        if let Some(clip) = self.residue_intersection(resolved, left, top, width, height)? {
            for (m, l) in tile.coverage.iter_mut().zip(&clip.coverage) {
                *m = ((u16::from(*m) * u16::from(*l) + 127) / 255) as u8;
            }
        }
        Ok(Some(tile))
    }

    /// The tile a shape with these device bounds occupies: shape ∩ clip ∩ target,
    /// rounded out to whole pixels.
    ///
    /// The same arithmetic `coverage_tile` does, without rasterising — which is what
    /// the GPU lane needs, since its coverage is drawn rather than computed.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    #[allow(clippy::arithmetic_side_effects, clippy::cast_precision_loss)]
    fn visible_tile(
        &self,
        bounds: (f32, f32, f32, f32),
        resolved: &ResolvedClip,
    ) -> Option<(i32, i32, u32, u32)> {
        let (x0, y0, x1, y1) = bounds;
        let vx0 = x0.max(resolved.rect.min.x).max(0.0);
        let vy0 = y0.max(resolved.rect.min.y).max(0.0);
        let vx1 = x1.min(resolved.rect.max.x).min(self.viewport.width as f32);
        let vy1 = y1.min(resolved.rect.max.y).min(self.viewport.height as f32);
        if vx0 >= vx1 || vy0 >= vy1 {
            return None;
        }
        let left = vx0.floor() as i32;
        let top = vy0.floor() as i32;
        let width = (vx1.ceil() as i32 - left).max(0) as u32;
        let height = (vy1.ceil() as i32 - top).max(0) as u32;
        (width > 0 && height > 0).then_some((left, top, width, height))
    }

    /// Whether this command takes the GPU lane.
    ///
    /// Four conditions, and every one of them is a measurement rather than a taste.
    ///
    /// **The caller asked for it.** [`Coverage::Gpu`] is a request; the rest decides
    /// where honouring it is a win.
    ///
    /// **No residue clip.** A non-rectangular clip multiplies into the coverage bytes on
    /// the CPU (`residue_product`), and there is no pass yet that does the same on the
    /// device (ADR 0016).
    ///
    /// **The tile is worth more than its triangles.** The GPU lane costs an outline's
    /// triangles *per placement, whatever the tile's size* — a nine-pixel glyph is
    /// 12.4 KB of them against ~150 bytes of coverage — so a page of small glyphs asked
    /// for 821 MB of vertices and was refused (ADR 0026).
    ///
    /// **And the cache is not worth using for this placement.** This is the condition
    /// ADR 0027 stated as a measured constant, ADR 0028 replaced with what the atlas
    /// *allows*, and ADR 0029 sharpened to what the atlas will *do* —
    /// [`CacheProspect::worth_caching`], which is the atlas's admission rule and the
    /// scene's census of placements in one answer. What the CPU lane has that the device
    /// has not is the cache: a tile rasterised once and read by every later placement and
    /// every later frame, which nothing this lane can do competes with. A tile the atlas
    /// refuses is rasterised into the scratch sheet again on every frame, and one the
    /// scene places a single time is rasterised, uploaded and read once — the cache's
    /// whole cost and none of its benefit. In both of those the device wins at every
    /// size measured.
    ///
    /// Measured on RADV at sixteen samples by `tests/lane_crossover.rs`, with the lane
    /// forced either way — a page of star outlines at 3 600 × 3 600, drawn to a texture
    /// target, milliseconds for the fastest of nine frames (a readback is excluded: its
    /// 15-20 ms of copy-out is paid identically by both lanes and hides the comparison):
    ///
    /// | tile | texels | atlas holds it | atlas refuses it |
    /// |---|---|---|---|
    /// | | | CPU / GPU | CPU / GPU |
    /// | 50 × 65 | 3 250 | **1.0** / 20.2 | 54.8 / **21.2** |
    /// | 200 × 260 | 52 000 | **0.4** / 16.0 | 35.5 / **15.0** |
    /// | 500 × 650 | 325 000 | **0.3** / 9.9 | 32.8 / **13.7** |
    /// | 700 × 910 | 637 000 | **0.2** / 11.1 | 26.0 / **12.6** |
    /// | 900 × 1170 | 1 053 000 | **0.4** / 13.3 | 33.9 / **15.0** |
    /// | 1 200 × 1 560 | 1 872 000 | — | 32.1 / **9.6** |
    ///
    /// The left column is one outline placed many times on the default atlas, the right
    /// the same page on an atlas too small to hold any of it. Twenty to sixty times the
    /// wrong answer on the left, two to three times the wrong answer on the right — and
    /// **no tile area distinguishes the columns**: the same 52 000-texel tile is in
    /// both, answered by different lanes. So the criterion is not a size at all.
    /// ADR 0027's 512 KiB sat below the atlas's admission threshold on the default
    /// budget, which is how one constant managed to be wrong in both directions at once.
    ///
    /// [`CacheProspect::worth_caching`]: crate::atlas::CacheProspect::worth_caching
    fn take_gpu_lane(
        &self,
        resolved: &ResolvedClip,
        cache: CacheProspect,
        width: u32,
        height: u32,
        triangles: usize,
    ) -> bool {
        if self.coverage != Coverage::Gpu || resolved.residues.is_some() || cache.worth_caching() {
            return false;
        }
        let area = u64::from(width).saturating_mul(u64::from(height));
        let triangle_bytes = (triangles as u64)
            .saturating_mul(3)
            .saturating_mul(crate::outline::WindingVertex::STRIDE);
        area >= triangle_bytes
    }

    /// Reserve a tile on the sheet and emit the quad that will sample it.
    ///
    /// `triangles` appends the shape's geometry in sheet space; it is handed the map
    /// from device pixels to sheet pixels, which is a translation and nothing else —
    /// the shape was already transformed into device space by the caller.
    #[allow(clippy::too_many_arguments)] // one draw's parameters, threaded once
    #[allow(clippy::cast_precision_loss)]
    #[allow(clippy::arithmetic_side_effects)] // a reserved tile fits the sheet, and the
    // sheet fits the device dimension: a corner cannot leave u32
    fn push_gpu_tile(
        &mut self,
        tile: (i32, i32, u32, u32),
        rule: Rule,
        color: quorra_scene::Color,
        resolved: &ResolvedClip,
        style: DrawStyle,
        mask: Option<u32>,
        triangles: impl FnOnce(&mut Vec<crate::outline::WindingVertex>, [f32; 2], [f32; 4]),
    ) -> Result<(), RenderError> {
        let (left, top, width, height) = tile;
        let (sx, sy) =
            self.scratch
                .reserve(width, height)
                .ok_or(RenderError::ScratchExhausted {
                    limit: self.scratch.max_height,
                })?;
        let origin = [sx as f32 - left as f32, sy as f32 - top as f32];
        let clip = [
            sx as f32,
            sy as f32,
            (sx + width) as f32,
            (sy + height) as f32,
        ];
        let mut vertices = Vec::new();
        triangles(&mut vertices, origin, clip);
        self.winding
            .push_tile(clip, rule == Rule::EvenOdd, &vertices);
        self.push_quad_instance(
            Point::new(left as f32, top as f32),
            width as f32,
            height as f32,
            sx as f32,
            sy as f32,
            1.0, // source: scratch, whichever lane drew it
            color,
            resolved.rect,
            style,
            mask,
        );
        Ok(())
    }

    /// Pack into scratch, charging is the caller's; splits from `push_scratch_quad`
    /// so residue planning can pack without emitting a quad.
    fn pack_scratch(&mut self, tile: &raster::CoverageMask) -> Result<(u32, u32), RenderError> {
        // Its own refusal, not the frame budget's: this one is about texture
        // capacity, and a message whose arithmetic contradicts itself costs the
        // reader the diagnosis (QUORRA_FEEDBACK.md §3 was exactly that report).
        let span = self.clock.start();
        let packed = self.scratch.pack(tile);
        self.clock.staging(span);
        packed.ok_or(RenderError::ScratchExhausted {
            limit: self.scratch.max_height,
        })
    }

    #[allow(clippy::cast_precision_loss)]
    #[allow(clippy::too_many_arguments)] // one draw's parameters, threaded once
    fn push_scratch_quad(
        &mut self,
        tile: &raster::CoverageMask,
        dest: Point,
        color: quorra_scene::Color,
        clip: Rect,
        style: DrawStyle,
        mask: Option<u32>,
    ) -> Result<(), RenderError> {
        let (sx, sy) = self.pack_scratch(tile)?;
        self.push_quad_instance(
            dest,
            tile.width as f32,
            tile.height as f32,
            sx as f32,
            sy as f32,
            1.0, // source: scratch
            color,
            clip,
            style,
            mask,
        );
        Ok(())
    }

    /// Charge one coverage tile, remembering how much of the sheet has been paid for
    /// tile by tile — the sheet's own extent is charged once at the end (ADR 0021), and
    /// this is what keeps that from charging twice for the same bytes.
    fn charge_tile(&mut self, width: u32, height: u32) -> Result<(), RenderError> {
        let bytes = u64::from(width).saturating_mul(u64::from(height));
        self.scratch_charged = self.scratch_charged.saturating_add(bytes);
        self.charge(bytes)
    }

    fn charge(&mut self, bytes: u64) -> Result<(), RenderError> {
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

    /// One instance of the analytic rectangle lane.
    ///
    /// `style` is a parameter rather than `self.style` because the lane now takes fills
    /// as well as [`Command::Rect`]s (ADR 0047), and a fill names its own pass through
    /// [`Compose`]: `Src` is §11.4.6's knockout, `DestOut` and `Plus` are its two stages
    /// asked for by name (ADR 0025). A `Rect` carries no compose mode and so passes the
    /// enclosing group's style, which is what this method used to read for itself.
    fn push_rect_instance(
        &mut self,
        rect: Rect,
        color: quorra_scene::Color,
        style: DrawStyle,
        mask: Option<u32>,
    ) {
        self.plan_mut()
            .mark([rect.min.x, rect.min.y, rect.max.x, rect.max.y]);
        let premultiplied = [
            color.r * color.a,
            color.g * color.a,
            color.b * color.a,
            color.a,
        ];
        for value in [rect.min.x, rect.min.y, rect.max.x, rect.max.y] {
            self.rect_instances.extend_from_slice(&value.to_le_bytes());
        }
        for value in premultiplied {
            self.rect_instances.extend_from_slice(&value.to_le_bytes());
        }
        self.note_batch(BatchKind::Rect, style, mask);
    }

    #[allow(clippy::too_many_arguments)] // one instance layout, one writer
    fn push_quad_instance(
        &mut self,
        dest: Point,
        width: f32,
        height: f32,
        tex_x: f32,
        tex_y: f32,
        source: f32,
        color: quorra_scene::Color,
        clip: Rect,
        style: DrawStyle,
        mask: Option<u32>,
    ) {
        self.plan_mut()
            .mark([dest.x, dest.y, dest.x + width, dest.y + height]);
        let premultiplied = [
            color.r * color.a,
            color.g * color.a,
            color.b * color.a,
            color.a,
        ];
        let values = [
            dest.x,
            dest.y,
            width,
            height,
            tex_x,
            tex_y,
            source,
            0.0,
            premultiplied[0],
            premultiplied[1],
            premultiplied[2],
            premultiplied[3],
            clip.min.x,
            clip.min.y,
            clip.max.x,
            clip.max.y,
        ];
        for value in values {
            self.quad_instances.extend_from_slice(&value.to_le_bytes());
        }
        self.note_batch(BatchKind::Quad, style, mask);
    }

    /// The plan currently under construction.
    fn plan_mut(&mut self) -> &mut LayerPlan {
        if self.current_plan == usize::MAX {
            &mut self.root
        } else {
            &mut self.layers[self.current_plan]
        }
    }

    /// Append an op to the current plan, and grow the plan's bounds to hold it
    /// (ADR 0036).
    ///
    /// Here rather than at the four call sites, because a site that forgot would give a
    /// plan a texture too small for what it draws — and the mark would be *clipped*,
    /// which is a plausible-looking wrong page rather than an error. A `Draw` is the
    /// exception and marks as its instances are pushed: a batch is a range, and the
    /// rectangles are the instances'.
    fn push_op(&mut self, op: Op) {
        match &op {
            Op::Image(image) => self.plan_mut().mark(image.dest),
            Op::Shaded(shaded) => self.plan_mut().mark(shaded.dest),
            // A child is the one op that may not be appended at all, so it goes through
            // the method that decides — from here, so that no call site can reach the
            // plain append and skip the decision. `ChildOp` is `Copy`.
            Op::Child(child) => return self.push_child(*child),
            Op::Draw(_) => {}
        }
        self.plan_mut().ops.push(op);
    }

    /// Append a child composite — unless the clip the composite will apply leaves the
    /// child no pixel of this plan to contribute to, in which case it is dropped
    /// (ADR 0041).
    ///
    /// The child's subtree has already been encoded when this runs, so what is saved is
    /// its *rendering*: a layer texture, a pass per plan below it, and the composite
    /// itself. [`Counters::layers_culled`] reports how often that happened, since a
    /// saving nobody counts is a saving nobody can check.
    ///
    /// **Why dropping it draws the same frame**, for each of the four things
    /// `composite.wgsl` can be. Write `b` for the backdrop the pass reads, `s` for the
    /// child's pixel and `w` for the group's constant alpha times its soft mask, its
    /// clip coverage and its clip residue. [`child_contribution`] establishes that at
    /// every pixel of this plan either `s = 0` or `w = 0` — the first outside the
    /// child's own marks, the second outside its clip rectangle — and both branches of
    /// each formula land on `b`:
    ///
    /// - §11.3.6, the ordinary composite: `co = as·(1−ab)·Cs + ab·(1−as)·Cb +
    ///   as·ab·B(Cb, Cs)` with `as = s.a·w = 0` is `ab·Cb`, and `ao = as + ab·(1−as)` is
    ///   `ab`. The pass writes back the backdrop it read.
    /// - §11.4.6 stage 1, the erase this group *is* when `compose == 1` (ADR 0033):
    ///   `P' = (1 − f) × P` with the group's alpha as the shape `f`, so
    ///   `b × (1 − s.a·w)` = `b`. **An erase weighted by a shape that is zero everywhere
    ///   erases nothing** — which is the case worth stating, because a wrong cull here
    ///   would show as a hole rather than as a missing mark.
    /// - §11.4.6 stage 2, the deposit: `P' = P + S`, so `b + s·w` = `b`.
    /// - §11.4.4, the non-isolated group: `mix(b, s, w)`. Its layer was seeded with a
    ///   texel-for-texel copy of this very accumulator and nothing wrote the accumulator
    ///   in between, so wherever the child marked nothing `s` *is* `b` and the
    ///   interpolation is `b` for any weight; wherever `w` is zero it is `b` again.
    ///   A seeded plan also takes its parent's region rather than its own (ADR 0038), so
    ///   this is the one case the compositor's own `region.meet` can never catch, and
    ///   the only place it can be caught is here.
    ///
    /// The staged pair and the non-isolated group cannot combine — `SceneBuilder`
    /// refuses `DestOut`/`Plus` on a group that is not isolated, because §11.4.4's seed
    /// would put the backdrop's alpha into the shape those stages read — so the second
    /// and third bullets are always about a group whose `s` is its own marks alone.
    ///
    /// [`Counters::layers_culled`]: crate::frame::Counters::layers_culled
    /// [`child_contribution`]: Self::child_contribution
    fn push_child(&mut self, child: ChildOp) {
        let Some(contribution) = self.child_contribution(&child) else {
            self.culled_layers = self.culled_layers.saturating_add(1);
            return;
        };
        self.plan_mut().mark(contribution);
        self.plan_mut().ops.push(Op::Child(child));
    }

    /// The rectangle a child layer can put on the plan that composites it — its own
    /// marks held to the clip the composite applies to them — or `None` when that
    /// rectangle holds no device pixel.
    ///
    /// Two ways to reach `None`, and they are different situations with the same answer:
    /// a child whose `bounds` are `None` **marked nothing at all**, so its texture is a
    /// cleared texel and `s = 0` everywhere; a child whose bounds miss its clip marked
    /// something the composite's `clip_coverage` will multiply by zero. The first is not
    /// necessarily an empty group — an image with a singular placement and a fill of an
    /// empty outline both draw nothing while being perfectly well-formed commands.
    ///
    /// **Emptiness is decided at pixel granularity, not on area**, because that is the
    /// granularity the composite works at: `clip_coverage` is the overlap of the whole
    /// pixel cell with the clip rectangle, and `s` is one colour for the whole pixel
    /// however little of it the child marked. A pixel `p` can therefore carry a
    /// contribution only if `[p, p+1)²` overlaps the bounds *and* the clip with positive
    /// area, and the integers that do lie in `[floor(min), ceil(max))` for each — so the
    /// test is that the intersection, rounded **out**, is empty. Rounding in instead
    /// would drop a real half-covered edge pixel; testing positive area instead would
    /// cull a bounds and a clip that abut at a fractional coordinate and still share a
    /// pixel between them.
    ///
    /// A plan marks nothing outside its bounds — every lane's rectangle is exactly what
    /// its instance draws (ADR 0036) — which is what makes `s = 0` outside them a fact
    /// about the texture rather than a hope.
    ///
    /// The lookup below cannot miss: `plan_child` hands back the index it has just
    /// pushed. Treating a miss as nothing to contribute is the safe direction regardless,
    /// since the compositor indexes that list without a bound of its own.
    fn child_contribution(&self, child: &ChildOp) -> Option<[f32; 4]> {
        let bounds = self.layers.get(child.layer)?.bounds?;
        let clip = child.clip_rect;
        let reach = [
            bounds[0].max(clip[0]),
            bounds[1].max(clip[1]),
            bounds[2].min(clip[2]),
            bounds[3].min(clip[3]),
        ];
        let holds_a_pixel =
            reach[0].floor() < reach[2].ceil() && reach[1].floor() < reach[3].ceil();
        holds_a_pixel.then_some(reach)
    }

    /// Extend the current batch, or start a new one on any switch of lane, style or
    /// mask — scene order is preserved by breaking batches, never by reordering.
    #[allow(clippy::cast_possible_truncation, clippy::arithmetic_side_effects)]
    fn note_batch(&mut self, kind: BatchKind, style: DrawStyle, mask: Option<u32>) {
        let index = match kind {
            BatchKind::Rect => (self.rect_instances.len() as u64 / RECT_INSTANCE_STRIDE) - 1,
            BatchKind::Quad => (self.quad_instances.len() as u64 / QUAD_INSTANCE_STRIDE) - 1,
        } as u32;
        if let Some(Op::Draw(last)) = self.plan_mut().ops.last_mut()
            && last.kind == kind
            && last.style == style
            && last.mask == mask
            && last.first + last.count == index
        {
            last.count += 1;
            return;
        }
        self.push_op(Op::Draw(Batch {
            kind,
            first: index,
            count: 1,
            style,
            mask,
        }));
    }
}

/// §11.3.5's mode numbering for the composite shader: `BlendMode`'s declaration
/// order, which follows the clause's own table.
pub(crate) fn blend_word(mode: BlendMode) -> u32 {
    match mode {
        BlendMode::Normal => 0,
        BlendMode::Multiply => 1,
        BlendMode::Screen => 2,
        BlendMode::Overlay => 3,
        BlendMode::Darken => 4,
        BlendMode::Lighten => 5,
        BlendMode::ColorDodge => 6,
        BlendMode::ColorBurn => 7,
        BlendMode::HardLight => 8,
        BlendMode::SoftLight => 9,
        BlendMode::Difference => 10,
        BlendMode::Exclusion => 11,
        BlendMode::Hue => 12,
        BlendMode::Saturation => 13,
        BlendMode::Color => 14,
        BlendMode::Luminosity => 15,
    }
}

#[cfg(test)]
mod tests {
    use quorra_scene::{
        Affine, BlendMode, Color, Compose, FillRule, Paint, Point, Rect, SceneBuilder, Segment,
    };

    use super::{BatchKind, encode};
    use crate::atlas::AtlasStore;
    use crate::error::RenderError;
    use crate::resources::ResourceStore;
    use crate::startup::Coverage;
    use crate::viewport::Viewport;

    fn no_resources() -> ResourceStore {
        ResourceStore::new(0)
    }

    fn empty_atlas() -> AtlasStore {
        AtlasStore::new(1024, 1024)
    }

    fn scene_with_one_rect() -> quorra_scene::Scene {
        let mut builder = SceneBuilder::new();
        builder
            .rect(
                Rect::new(Point::new(1.0, 2.0), Point::new(3.0, 5.0)),
                Affine::IDENTITY,
                Color::new(1.0, 0.5, 0.0, 0.5),
                None,
                None,
            )
            .expect("valid input");
        builder.finish()
    }

    /// One rect, identity viewport: one instance, one rect batch, premultiplied
    /// colour at bytes 16..32.
    #[test]
    fn encodes_one_instance_with_premultiplied_color() {
        let scene = scene_with_one_rect();
        let viewport = Viewport::full(10, 10, Affine::IDENTITY);
        let encoded = encode(
            &scene,
            &viewport,
            u64::MAX,
            4096,
            &no_resources(),
            &mut empty_atlas(),
            Some(16),
            Coverage::Cpu,
            false,
        )
        .expect("within budget");
        assert_eq!(encoded.commands, 1);
        assert_eq!(encoded.rect_instances.len(), 32);
        assert_eq!(encoded.root.ops.len(), 1);
        assert!(matches!(
            encoded.root.ops[0],
            super::Op::Draw(super::Batch {
                kind: BatchKind::Rect,
                ..
            })
        ));
        let read_f32 = |offset: usize| {
            let bytes: [u8; 4] = encoded.rect_instances[offset..offset + 4]
                .try_into()
                .expect("in bounds");
            f32::from_le_bytes(bytes)
        };
        assert!((read_f32(16) - 0.5).abs() < 1e-6);
        assert!((read_f32(20) - 0.25).abs() < 1e-6);
        assert!((read_f32(24) - 0.0).abs() < 1e-6);
        assert!((read_f32(28) - 0.5).abs() < 1e-6);
    }

    /// An oblique rectangle no longer refuses: it takes the path lane and comes back
    /// as a scratch quad.
    #[test]
    fn oblique_rect_takes_the_path_lane() {
        let mut builder = SceneBuilder::new();
        let shear = Affine {
            a: 1.0,
            b: 0.3,
            c: 0.0,
            d: 1.0,
            e: 2.0,
            f: 2.0,
        };
        builder
            .rect(
                Rect::new(Point::new(0.0, 0.0), Point::new(4.0, 4.0)),
                shear,
                Color::new(0.0, 0.0, 0.0, 1.0),
                None,
                None,
            )
            .expect("valid rect");
        let scene = builder.finish();
        let viewport = Viewport::full(16, 16, Affine::IDENTITY);
        let encoded = encode(
            &scene,
            &viewport,
            u64::MAX,
            4096,
            &no_resources(),
            &mut empty_atlas(),
            Some(16),
            Coverage::Cpu,
            false,
        )
        .expect("drawable since M5");
        assert_eq!(encoded.root.ops.len(), 1);
        assert!(matches!(
            encoded.root.ops[0],
            super::Op::Draw(super::Batch {
                kind: BatchKind::Quad,
                ..
            })
        ));
        assert!(encoded.scratch.is_some());
    }

    /// A store holding one outline: the four axis-aligned edges of `1,2 → 3,5`, which
    /// is what `quorra_scene::axis_aligned_rect` recognises and what the analytic lane
    /// exists for.
    fn store_with_a_rectangle() -> (ResourceStore, quorra_scene::OutlineId) {
        let mut resources = ResourceStore::new(4_096);
        let outline = resources
            .upload_outline(&[
                Segment::MoveTo(Point::new(1.0, 2.0)),
                Segment::LineTo(Point::new(3.0, 2.0)),
                Segment::LineTo(Point::new(3.0, 5.0)),
                Segment::LineTo(Point::new(1.0, 5.0)),
                Segment::Close,
            ])
            .expect("a rectangle within the store's budget");
        (resources, outline)
    }

    /// One solid fill of `outline` under `transform`, and nothing else.
    fn scene_filling(
        outline: quorra_scene::OutlineId,
        transform: Affine,
        rule: FillRule,
    ) -> quorra_scene::Scene {
        let mut builder = SceneBuilder::new();
        builder
            .fill(
                outline,
                transform,
                rule,
                Paint::Solid(Color::new(1.0, 0.5, 0.0, 0.5)),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .expect("valid input");
        builder.finish()
    }

    /// A solid fill whose outline is four axis-aligned edges takes the rectangle lane
    /// (ADR 0047) — the same instance bytes `Command::Rect` produces for the same mark,
    /// with no atlas key and no coverage tile behind it.
    #[test]
    fn a_solid_fill_of_a_rectangle_takes_the_rectangle_lane() {
        let (resources, outline) = store_with_a_rectangle();
        let scene = scene_filling(outline, Affine::IDENTITY, FillRule::NonZero);
        let viewport = Viewport::full(10, 10, Affine::IDENTITY);
        let encoded = encode(
            &scene,
            &viewport,
            u64::MAX,
            4096,
            &resources,
            &mut empty_atlas(),
            Some(16),
            Coverage::Cpu,
            false,
        )
        .expect("within budget");
        assert!(matches!(
            encoded.root.ops.as_slice(),
            [super::Op::Draw(super::Batch {
                kind: BatchKind::Rect,
                count: 1,
                ..
            })]
        ));
        assert!(encoded.quad_instances.is_empty());
        assert!(encoded.scratch.is_none());
        assert_eq!(encoded.atlas_distinct_keys, 0);
        assert_eq!(encoded.tiles, 0);

        // The bytes are `Command::Rect`'s for the same rectangle and colour: this is
        // one lane reached through two doors, not two lanes that agree.
        let same_mark = {
            let mut builder = SceneBuilder::new();
            builder
                .rect(
                    Rect::new(Point::new(1.0, 2.0), Point::new(3.0, 5.0)),
                    Affine::IDENTITY,
                    Color::new(1.0, 0.5, 0.0, 0.5),
                    None,
                    None,
                )
                .expect("valid input");
            builder.finish()
        };
        let reference = encode(
            &same_mark,
            &viewport,
            u64::MAX,
            4096,
            &no_resources(),
            &mut empty_atlas(),
            Some(16),
            Coverage::Cpu,
            false,
        )
        .expect("within budget");
        assert_eq!(encoded.rect_instances, reference.rect_instances);

        // And the fill rule cannot change it: one closed subpath of four corners bounds
        // the same region under §8.5.3.3.2 and §8.5.3.3.3, which is why the lane does
        // not ask.
        let even_odd = encode(
            &scene_filling(outline, Affine::IDENTITY, FillRule::EvenOdd),
            &viewport,
            u64::MAX,
            4096,
            &resources,
            &mut empty_atlas(),
            Some(16),
            Coverage::Cpu,
            false,
        )
        .expect("within budget");
        assert_eq!(even_odd.rect_instances, encoded.rect_instances);
    }

    /// The lane's conditions bite: an oblique transform makes the same outline a
    /// parallelogram, which `rect.wgsl` cannot express, so it keeps the path lane.
    #[test]
    fn an_oblique_fill_of_a_rectangle_keeps_the_path_lane() {
        let (resources, outline) = store_with_a_rectangle();
        let shear = Affine {
            a: 1.0,
            b: 0.3,
            c: 0.0,
            d: 1.0,
            e: 2.0,
            f: 2.0,
        };
        let encoded = encode(
            &scene_filling(outline, shear, FillRule::NonZero),
            &Viewport::full(16, 16, Affine::IDENTITY),
            u64::MAX,
            4096,
            &resources,
            &mut empty_atlas(),
            Some(16),
            Coverage::Cpu,
            false,
        )
        .expect("drawable");
        assert!(encoded.rect_instances.is_empty());
        assert!(!encoded.quad_instances.is_empty());
    }

    /// The budget is checked before allocation, and the error names both numbers.
    #[test]
    fn budget_is_checked_before_allocation() {
        let scene = scene_with_one_rect();
        let viewport = Viewport::full(10, 10, Affine::IDENTITY);
        match encode(
            &scene,
            &viewport,
            16,
            4096,
            &no_resources(),
            &mut empty_atlas(),
            Some(16),
            Coverage::Cpu,
            false,
        ) {
            Err(RenderError::FrameBudgetExceeded { needed, budget }) => {
                assert_eq!(needed, 96);
                assert_eq!(budget, 16);
            }
            other => panic!("expected FrameBudgetExceeded, got {other:?}"),
        }
    }

    /// A blank scene encodes to zero instances and zero batches, without error.
    #[test]
    fn blank_scene_encodes_to_nothing() {
        let scene = SceneBuilder::new().finish();
        let viewport = Viewport::full(10, 10, Affine::IDENTITY);
        let encoded = encode(
            &scene,
            &viewport,
            u64::MAX,
            4096,
            &no_resources(),
            &mut empty_atlas(),
            Some(16),
            Coverage::Cpu,
            false,
        )
        .expect("blank is legitimate");
        assert_eq!(encoded.commands, 0);
        assert!(encoded.root.ops.is_empty());
        assert!(encoded.scratch.is_none());
    }
}
