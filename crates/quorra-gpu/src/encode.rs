//! Phase 1 of a frame: classify into lanes, resolve clips, rasterise coverage, count,
//! then lay out instance data.
//!
//! One CPU walk over the scene's commands (PLAN.md Part 1 §1.2), sorting each into
//! the cheapest lane that draws it exactly:
//!
//! - **rectangle** — axis-aligned rect, axis-preserving transform, fully rectangular
//!   clip: analytic coverage in the shader, clip applied by intersection here
//!   (ADR 0007), zero per-pixel clip cost;
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

use std::collections::HashSet;
use std::sync::Arc;

use quorra_scene::{
    Affine, BlendMode, ClipId, Command, Compose, FillRule, ImageFilter, ImageId, MaskId, MaskKind,
    OutlineId, Paint, Point, Rect, Scene, ShadingKind,
};

use crate::atlas::{AtlasStore, CacheProspect, GlyphKey, GlyphPlacement};
use crate::census::Census;
use crate::error::RenderError;
use crate::instrument::EncodeClock;
use crate::keyhash::FastSet;
use crate::raster::{self, DeviceTransform, Polyline, Rule};
use crate::resources::ResourceStore;
use crate::startup::Coverage;
use crate::viewport::Viewport;

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

/// A clip rectangle that admits everything, for unclipped instances.
const OPEN_CLIP: [f32; 4] = [-1.0e9, -1.0e9, 1.0e9, 1.0e9];

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

/// One image draw (ISO 32000-2 §8.9.5), executed as a single uniform-driven quad.
///
/// The fragment shader maps device pixels back through `inv`, so the quad only has
/// to cover the footprint; an axis-preserving placement gets analytic edge coverage
/// from `image_rect`, an oblique one paints where centres land inside the unit
/// square (ADR 0011 carries both decisions).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ImageOp {
    /// The resident image's raw id.
    pub image: u32,
    /// Inverse of the unit-square → device transform, §8.3.3 coefficient order.
    pub inv: [f32; 6],
    /// The footprint's device bounding rectangle (exact when `axis_aligned`).
    pub image_rect: [f32; 4],
    /// The quad drawn: footprint ∩ clip ∩ target, at pixel bounds.
    pub dest: [f32; 4],
    /// The resolved clip rectangle.
    pub clip: [f32; 4],
    /// Where a rasterised residue clip sits in the frame's scratch, if one applies;
    /// its tile spans exactly `dest`.
    pub residue_origin: Option<[f32; 2]>,
    /// Whether the placement preserves axes (analytic edges).
    pub axis_aligned: bool,
    /// The command's constant alpha (§11.6.4.3).
    pub alpha: f32,
    /// The placement's resolved filter: `true` for linear (§4.5, integration
    /// note 1).
    pub linear: bool,
    pub style: DrawStyle,
    pub mask: Option<u32>,
}

/// Which texture paints a [`ShadedOp`].
#[derive(Debug, Clone, Copy)]
pub(crate) enum PaintSource {
    /// A 256×1 pre-sampled colour ramp (raw ramp id).
    Ramp(u32),
    /// A pre-rasterised mesh (raw mesh id), sampled at absolute device pixels.
    Mesh(u32),
}

/// One shading or mesh draw (ISO 32000-2 §8.7.4.5), a single uniform-driven quad
/// over a coverage source: a scratch tile for a rasterised shape, or the analytic
/// rectangle for the rect-hinted case.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ShadedOp {
    pub paint: PaintSource,
    /// Inverse of the shading-space → device transform (identity for meshes, which
    /// are already device-space).
    pub inv: [f32; 6],
    /// 0 axial, 1 radial, 2 mesh — the shader's kind word.
    pub kind_word: f32,
    /// Bit 0: extend beyond the start; bit 1: beyond the end (§8.7.4.5.2/.3).
    pub extend_bits: u32,
    /// Axial/radial: start.xy, end.xy in shading space. Mesh: left, top in device
    /// pixels.
    pub geo0: [f32; 4],
    /// Radial: start radius, end radius.
    pub geo1: [f32; 4],
    /// The quad drawn; when coverage comes from scratch, exactly the tile's bounds.
    pub dest: [f32; 4],
    /// The coverage tile's origin in scratch, or `None` for the analytic rectangle.
    pub coverage_origin: Option<[f32; 2]>,
    /// The analytic coverage rectangle (the shape itself), used when
    /// `coverage_origin` is `None`.
    pub coverage_rect: [f32; 4],
    pub clip: [f32; 4],
    pub style: DrawStyle,
    pub mask: Option<u32>,
}

/// The shading-space geometry of a non-solid paint, resolved once per command.
#[derive(Debug, Clone, Copy)]
struct ShadedGeometry {
    paint: PaintSource,
    kind_word: f32,
    extend_bits: u32,
    geo0: [f32; 4],
    geo1: [f32; 4],
    inv: [f32; 6],
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
    /// The group's soft mask, as a mask index.
    pub mask: Option<u32>,
    /// §11.4.5's isolated group (the ordinary case) or §11.4.4's non-isolated one,
    /// whose layer is seeded with the backdrop and interpolated back onto it
    /// (ADR 0019). The implicit one-element groups §11.3.5 needs for a blended
    /// element are isolated: the wrapper is a device trick, not a PDF group.
    pub isolated: bool,
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

/// The frame's scratch coverage image: every uncached mask, shelf-packed into one
/// R8 upload.
#[derive(Debug)]
pub(crate) struct Scratch {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
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
    pub distinct_outlines: u32,
    pub atlas_distinct_keys: u32,
    pub segments: u32,
    /// Commands the walk rejected for reaching no pixel of the target.
    pub commands_culled: u32,
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
    /// What the walk above spent its time on, when the caller asked for the
    /// subdivision (ADR 0023); empty otherwise.
    pub encode_phases: EncodeClock,
}

/// A resolved clip chain: the intersection of its rectangular links, plus the chain
/// of non-rectangular links (the residue) that must multiply into a coverage mask.
#[derive(Debug, Clone)]
struct ResolvedClip {
    rect: Rect,
    residues: Option<Arc<ResidueLink>>,
}

#[derive(Debug)]
struct ResidueLink {
    clip: ClipId,
    parent: Option<Arc<ResidueLink>>,
}

fn open_clip() -> ResolvedClip {
    ResolvedClip {
        rect: Rect::new(
            Point::new(OPEN_CLIP[0], OPEN_CLIP[1]),
            Point::new(OPEN_CLIP[2], OPEN_CLIP[3]),
        ),
        residues: None,
    }
}

/// Chains resolved so far this frame, memoised across shared prefixes — the caller's
/// worst page holds 3 608 chains.
struct ClipResolver {
    resolved: Vec<Option<ResolvedClip>>,
}

impl ClipResolver {
    fn new(clip_count: usize) -> Self {
        Self {
            resolved: vec![None; clip_count],
        }
    }

    /// Iterative on purpose: chains are deep on real pages and a recursive walk
    /// would put the depth on the stack. Cycles cannot occur — a parent id is always
    /// smaller than its child's, by construction in `SceneBuilder::clip`.
    fn resolve(
        &mut self,
        id: ClipId,
        scene: &Scene,
        viewport: &Viewport<'_>,
        resources: &ResourceStore,
    ) -> Result<ResolvedClip, RenderError> {
        let mut pending: Vec<ClipId> = Vec::new();
        let mut cursor = Some(id);
        let mut inherited: Option<ResolvedClip> = None;
        while let Some(link) = cursor {
            if let Some(resolved) = &self.resolved[link.0 as usize] {
                inherited = Some(resolved.clone());
                break;
            }
            pending.push(link);
            cursor = scene.clips()[link.0 as usize].parent;
        }
        let mut current = inherited.unwrap_or_else(open_clip);
        while let Some(link) = pending.pop() {
            let def = &scene.clips()[link.0 as usize];
            let stored = resources
                .outline(def.outline)
                .ok_or(RenderError::UnknownOutline {
                    outline: def.outline,
                })?;
            let to_device = compose(def.transform, viewport);
            let rect_link = if transform_preserves_axes(&to_device) {
                stored.rect_hint
            } else {
                None
            };
            current = match rect_link {
                Some(rect) => {
                    let p0 = apply(&to_device, rect.min);
                    let p1 = apply(&to_device, rect.max);
                    let device_rect = Rect::new(
                        Point::new(p0.x.min(p1.x), p0.y.min(p1.y)),
                        Point::new(p0.x.max(p1.x), p0.y.max(p1.y)),
                    );
                    ResolvedClip {
                        rect: current.rect.intersection(device_rect),
                        residues: current.residues.clone(),
                    }
                }
                // Not a rectangle under this transform: a residue link, multiplied
                // into coverage masks at draw time (M5).
                None => ResolvedClip {
                    rect: current.rect,
                    residues: Some(Arc::new(ResidueLink {
                        clip: link,
                        parent: current.residues.clone(),
                    })),
                },
            };
            self.resolved[link.0 as usize] = Some(current.clone());
        }
        Ok(current)
    }
}

/// The frame's scratch shelf packer: coverage tiles packed into one growing R8
/// image, uploaded once.
struct ScratchPacker {
    width: u32,
    max_height: u32,
    shelves: Vec<(u32, u32, u32)>, // (y, height, cursor)
    next_y: u32,
    data: Vec<u8>,
    /// Tiles placed on the sheet, for `Counters::tiles` — the count both lanes feed,
    /// since `reserve` is the one door onto the sheet.
    placed: u32,
}

impl ScratchPacker {
    fn new(width: u32, max_height: u32) -> Self {
        Self {
            width,
            max_height,
            shelves: Vec::new(),
            next_y: 0,
            data: Vec::new(),
            placed: 0,
        }
    }

    /// Reserve a tile's place on the sheet, without writing anything into it.
    ///
    /// The shelf arithmetic, alone. Both producers go through it — the CPU lane then
    /// copies its bytes in, the GPU lane hands the position to a pass that draws there
    /// — which is what lets one sheet hold both kinds of tile without either knowing
    /// the other exists (ADR 0016).
    #[allow(clippy::arithmetic_side_effects)] // bounded by width/max_height checks
    fn reserve(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        if width == 0 || height == 0 || width > self.width {
            return None;
        }
        self.placed = self.placed.saturating_add(1);
        self.shelves
            .iter_mut()
            .find(|(_, shelf_height, cursor)| {
                *shelf_height >= height
                    && *shelf_height <= height.saturating_mul(2)
                    && cursor + width <= self.width
            })
            .map(|(y, _, cursor)| {
                let position = (*cursor, *y);
                *cursor += width;
                position
            })
            .or_else(|| {
                if self.next_y + height <= self.max_height {
                    let y = self.next_y;
                    self.next_y += height;
                    self.shelves.push((y, height, width));
                    Some((0, y))
                } else {
                    None
                }
            })
    }

    /// Pack a mask's bytes; `None` when the frame's scratch would outgrow the
    /// texture dimension limit (the byte budget was already charged by the caller).
    #[allow(clippy::arithmetic_side_effects)] // bounded by width/max_height checks
    fn pack(&mut self, mask: &raster::CoverageMask) -> Option<(u32, u32)> {
        let position = self.reserve(mask.width, mask.height)?;
        let row_bytes = self.width as usize;
        let needed_rows = (position.1 + mask.height) as usize;
        if self.data.len() < needed_rows * row_bytes {
            self.data.resize(needed_rows * row_bytes, 0);
        }
        for row in 0..mask.height as usize {
            let src = &mask.coverage[row * mask.width as usize..(row + 1) * mask.width as usize];
            let dst_start = (position.1 as usize + row) * row_bytes + position.0 as usize;
            self.data[dst_start..dst_start + mask.width as usize].copy_from_slice(src);
        }
        Some(position)
    }

    /// The packed sheet, or `None` when nothing was placed on it.
    ///
    /// **Narrowed to the width the shelves actually reached** (ADR 0021). The packing
    /// width is the device's maximum dimension because a narrow one refuses real pages
    /// (the caller's feedback §3), but every tile sits left of the widest shelf cursor,
    /// so everything to the right of it is a texture nobody wrote and a texture nobody
    /// reads — 16 384 texels of it on this machine. Narrowing moves no tile: the
    /// coordinates the lanes recorded are all inside the kept region.
    ///
    /// `data` is padded to the whole sheet when the CPU lane wrote any of it, and left
    /// **empty** when every tile on the sheet is the GPU lane's — the device then
    /// clears rather than uploads, and the distinction is the one bit it needs.
    fn finish(mut self) -> Option<Scratch> {
        if self.next_y == 0 {
            return None;
        }
        let used = self
            .shelves
            .iter()
            .map(|(_, _, cursor)| *cursor)
            .max()
            .unwrap_or(self.width)
            .clamp(1, self.width);
        if !self.data.is_empty() {
            // Rows the CPU lane actually wrote, counted in the *packing* stride before
            // compaction restrides them.
            let written = self
                .data
                .len()
                .checked_div(self.width as usize)
                .unwrap_or(0);
            self.compact_rows(used);
            // **Cut the tail before growing the sheet.** Compaction moves each row left
            // and leaves the old wide layout's bytes behind it, so the buffer is still
            // as long as it was and everything past `written × used` is stale coverage
            // rather than blank sheet. Resizing straight to the sheet's extent keeps
            // whatever of that tail happens to fall inside it — which is nothing at all
            // while every shelf holds CPU tiles (they write their own rows), and is a
            // page of somebody else's marks the moment a shelf below them belongs to a
            // lane that writes its rows on the device instead. The caller's
            // `transparency_group.pdf` drew 136 410 texels of another shape's coverage
            // that way, in horizontal streaks across the rows under its last CPU tile.
            self.data.truncate(written.saturating_mul(used as usize));
            // Both extents are bounded by the device dimension, so the product is far
            // inside a `usize`; saturating says so rather than relying on it.
            self.data
                .resize((used as usize).saturating_mul(self.next_y as usize), 0);
        }
        Some(Scratch {
            width: used,
            height: self.next_y,
            data: self.data,
        })
    }

    /// Restride the written rows from the packing width down to `used`, in place.
    ///
    /// Rows move left and never right, and row `r`'s destination start is always below
    /// its source start, so a forward copy cannot overwrite bytes it has yet to read.
    fn compact_rows(&mut self, used: u32) {
        if used >= self.width {
            return;
        }
        let (from, to) = (self.width as usize, used as usize);
        // `from` is the packing width, which `new` takes from the device dimension and
        // `reserve` refuses to place anything into when it is zero.
        let rows = self.data.len().checked_div(from).unwrap_or(0);
        for row in 1..rows {
            let source = row.saturating_mul(from);
            let destination = row.saturating_mul(to);
            self.data
                .copy_within(source..source.saturating_add(to), destination);
        }
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
/// control hull [`outline_device_bounds`] measures.
const CULL_MARGIN: f32 = 2.0;

/// The composed-transform bounding box of an outline's control points — a bound on
/// the curve itself, by the convex-hull property of Béziers.
fn outline_device_bounds(
    segments: &[quorra_scene::Segment],
    t: &DeviceTransform,
) -> Option<(f32, f32, f32, f32)> {
    use quorra_scene::Segment;
    let mut bounds: Option<(f32, f32, f32, f32)> = None;
    let mut extend = |p: Point| {
        let q = apply(t, p);
        bounds = Some(match bounds {
            None => (q.x, q.y, q.x, q.y),
            Some((x0, y0, x1, y1)) => (x0.min(q.x), y0.min(q.y), x1.max(q.x), y1.max(q.y)),
        });
    };
    for segment in segments {
        match *segment {
            Segment::MoveTo(p) | Segment::LineTo(p) => extend(p),
            Segment::CubicTo { c1, c2, to } => {
                extend(c1);
                extend(c2);
                extend(to);
            }
            Segment::Close => {}
        }
    }
    bounds
}

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
    distinct_outlines: HashSet<u32>,
    atlas_keys: FastSet<GlyphKey>,
    used_images: HashSet<u32>,
    used_ramps: HashSet<u32>,
    used_meshes: HashSet<u32>,
    segments: u64,
    /// Commands that could reach no pixel of the target, and so were never built.
    culled: u32,
    atlas_pressure: bool,
    /// See `Encoded::atlas_requested_bytes`.
    atlas_requested_bytes: u64,
    /// Sheet bytes already charged tile by tile, so the sheet's own extent can be
    /// charged once at the end without paying twice (ADR 0021).
    scratch_charged: u64,
    /// What encode spent its time on, when the caller asked (ADR 0023).
    clock: EncodeClock,
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
        // how many *other* fills share its tile, which is not knowable from the fill.
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
        // The scratch sheet spans the full device dimension both ways: its *byte*
        // cost is charged tile by tile against the frame budget, so the dimension
        // is capacity, not commitment — and a 2048-texel width refused real pages
        // whose coverage was well inside the budget (QUORRA_FEEDBACK.md §3).
        scratch: ScratchPacker::new(max_dimension, max_dimension),
        rect_instances: Vec::new(),
        quad_instances: Vec::new(),
        current_plan: usize::MAX,
        root: LayerPlan::default(),
        layers: Vec::new(),
        mask_plans: (0..scene.masks().len()).map(|_| None).collect(),
        style: DrawStyle::Over,
        budget: frame_budget_bytes,
        spent: needed,
        distinct_outlines: HashSet::new(),
        atlas_keys: FastSet::default(),
        used_images: HashSet::new(),
        used_ramps: HashSet::new(),
        used_meshes: HashSet::new(),
        segments: 0,
        culled: 0,
        atlas_pressure: false,
        atlas_requested_bytes: 0,
        scratch_charged: 0,
        clock: EncodeClock::new(instrument),
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

    let mut distinct = HashSet::new();
    for resolved in encoder.clips.resolved.iter().flatten() {
        distinct.insert([
            resolved.rect.min.x.to_bits(),
            resolved.rect.min.y.to_bits(),
            resolved.rect.max.x.to_bits(),
            resolved.rect.max.y.to_bits(),
        ]);
    }

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
        clip_distinct_regions: u32::try_from(distinct.len()).unwrap_or(u32::MAX),
        distinct_outlines: u32::try_from(encoder.distinct_outlines.len()).unwrap_or(u32::MAX),
        atlas_distinct_keys: u32::try_from(encoder.atlas_keys.len()).unwrap_or(u32::MAX),
        segments: u32::try_from(encoder.segments).unwrap_or(u32::MAX),
        commands_culled: encoder.culled,
        winding,
        atlas_pressure: encoder.atlas_pressure,
        atlas_requested_bytes: encoder.atlas_requested_bytes,
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
        if let Some((x0, y0, x1, y1)) = outline_device_bounds(&stored.segments, &to_device)
            && self.culled((x0 - reach, y0 - reach, x1 + reach, y1 + reach), &resolved)
        {
            return Ok(());
        }
        if blend != BlendMode::Normal {
            // §11.3.5 for a single element: an implicit one-element group.
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
            self.push_op(Op::Child(ChildOp {
                layer: child,
                mode: blend_word(blend),
                alpha: 1.0,
                clip_rect: OPEN_CLIP,
                residue_rect: [0.0; 4],
                residue_origin: [0.0; 2],
                mask,
                isolated: true,
            }));
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

    /// A chain's residue links over a region, intersected into one coverage tile —
    /// `None` when the chain has none. The caller charges the region's bytes.
    ///
    /// **The links intersect; they do not multiply** (ADR 0030). ISO 32000-2 §8.5.4 is
    /// explicit that a chain is not a stack of boundaries at all:
    ///
    /// > After the path has been painted, the clipping path in the graphics state shall
    /// > be set to the intersection of the current clipping path and the newly
    /// > constructed path.
    ///
    /// One region, arrived at by intersecting paths — so rasterising each link on its
    /// own is our implementation's convenience, and the rule that puts them back
    /// together owes the clause an intersection. `min` is that: idempotent, so restating
    /// a clip changes nothing the way intersecting a region with itself changes nothing,
    /// and exact wherever two boundaries coincide or nest.
    fn residue_intersection(
        &mut self,
        resolved: &ResolvedClip,
        left: i32,
        top: i32,
        width: u32,
        height: u32,
    ) -> Result<Option<raster::CoverageMask>, RenderError> {
        let mut combined: Option<raster::CoverageMask> = None;
        let mut residue = resolved.residues.clone();
        while let Some(link) = residue.take() {
            let def = &self.scene.clips()[link.clip.0 as usize];
            let stored =
                self.resources
                    .outline(def.outline)
                    .ok_or(RenderError::UnknownOutline {
                        outline: def.outline,
                    })?;
            let link_transform = compose(def.transform, self.viewport);
            let span = self.clock.start();
            let link_polylines = raster::flatten(&stored.segments, link_transform);
            let link_rule = match def.rule {
                FillRule::NonZero => Rule::NonZero,
                FillRule::EvenOdd => Rule::EvenOdd,
            };
            let link_mask = raster::fill_mask(&link_polylines, link_rule, left, top, width, height);
            self.clock.geometry(span);
            combined = Some(match combined {
                None => link_mask,
                Some(mut base) => {
                    for (m, l) in base.coverage.iter_mut().zip(&link_mask.coverage) {
                        *m = (*m).min(*l);
                    }
                    base
                }
            });
            residue =
                Arc::try_unwrap(link).map_or_else(|link| link.parent.clone(), |link| link.parent);
        }
        Ok(combined)
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
            let p0 = apply(&to_device, rect.min);
            let p1 = apply(&to_device, rect.max);
            let device_rect = Rect::new(
                Point::new(p0.x.min(p1.x), p0.y.min(p1.y)),
                Point::new(p0.x.max(p1.x), p0.y.max(p1.y)),
            )
            .intersection(resolved.rect)
            // And with the target, which costs no pixel any coverage: `target_rect`
            // has integer corners, so an edge it introduces falls exactly on a pixel
            // boundary. This is the analytic lane's whole cull — the region it draws
            // decides it, with no margin, because nothing here rounds outwards.
            .intersection(self.visible);
            if device_rect.is_empty() {
                // Clipped to nothing or off the target: draws nothing, legitimately.
                self.note_culled();
                return Ok(());
            }
            self.push_rect_instance(device_rect, color, mask);
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
        let bounds = outline_device_bounds(&stored.segments, &to_device);
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
        if let (None, Some(placement)) = (resolved.residues.as_ref(), cache.placement()) {
            return self.push_glyph(
                fill.outline,
                &fill.to_device,
                &placement,
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
        self.push_op(Op::Child(ChildOp {
            layer: child,
            mode: blend_word(blend),
            alpha: 1.0,
            clip_rect: OPEN_CLIP,
            residue_rect: [0.0; 4],
            residue_origin: [0.0; 2],
            mask,
            isolated: true,
        }));
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

    /// The image arm (ISO 32000-2 §8.9.5): one uniform-driven quad per placement,
    /// with a non-Normal blend through an implicit child, as fills take it.
    #[allow(clippy::too_many_arguments)] // one command's fields, destructured once
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    #[allow(clippy::arithmetic_side_effects, clippy::cast_precision_loss)]
    fn encode_image(
        &mut self,
        image: ImageId,
        transform: Affine,
        alpha: f32,
        filter: ImageFilter,
        clip: Option<ClipId>,
        blend: BlendMode,
        mask: Option<MaskId>,
    ) -> Result<(), RenderError> {
        let mask = self.use_mask(mask)?;
        if blend != BlendMode::Normal && self.style == DrawStyle::Over {
            // §11.3.5 for a single element: an implicit one-element group (the same
            // degeneracy argument as in `encode_fill` skips it under knockout).
            let child = self.plan_child(|encoder| {
                encoder.encode_image(
                    image,
                    transform,
                    alpha,
                    filter,
                    clip,
                    BlendMode::Normal,
                    None,
                )
            })?;
            self.push_op(Op::Child(ChildOp {
                layer: child,
                mode: blend_word(blend),
                alpha: 1.0,
                clip_rect: OPEN_CLIP,
                residue_rect: [0.0; 4],
                residue_origin: [0.0; 2],
                mask,
                isolated: true,
            }));
            return Ok(());
        }
        if self.resources.image(image).is_none() {
            return Err(RenderError::UnknownImage { image });
        }
        self.used_images.insert(image.0);
        let resolved = self.resolve_clip(clip)?;
        let to_device = compose(transform, self.viewport);
        let Some(inverse) = transform.then(self.viewport.transform).invert() else {
            // A singular placement collapses the unit square to a zero-area set:
            // nothing to paint, and no way to map pixels back into it.
            return Ok(());
        };
        let corners = [
            apply(&to_device, Point::new(0.0, 0.0)),
            apply(&to_device, Point::new(1.0, 0.0)),
            apply(&to_device, Point::new(0.0, 1.0)),
            apply(&to_device, Point::new(1.0, 1.0)),
        ];
        let bx0 = corners.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
        let by0 = corners.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
        let bx1 = corners
            .iter()
            .map(|p| p.x)
            .fold(f32::NEG_INFINITY, f32::max);
        let by1 = corners
            .iter()
            .map(|p| p.y)
            .fold(f32::NEG_INFINITY, f32::max);
        // The quad drawn: footprint ∩ clip ∩ target, expanded to pixel bounds so
        // partially covered edge pixels get their fragments.
        let vx0 = bx0.max(resolved.rect.min.x).max(0.0);
        let vy0 = by0.max(resolved.rect.min.y).max(0.0);
        let vx1 = bx1.min(resolved.rect.max.x).min(self.viewport.width as f32);
        let vy1 = by1
            .min(resolved.rect.max.y)
            .min(self.viewport.height as f32);
        if vx0 >= vx1 || vy0 >= vy1 {
            // Clipped to nothing or off the target: draws nothing, legitimately.
            // Exact, like the analytic rectangle lane — this *is* the region drawn.
            self.note_culled();
            return Ok(());
        }
        let left = vx0.floor() as i32;
        let top = vy0.floor() as i32;
        let width = (vx1.ceil() as i32 - left).max(1) as u32;
        let height = (vy1.ceil() as i32 - top).max(1) as u32;
        let residue_origin = if resolved.residues.is_some() {
            self.charge_tile(width, height)?;
            match self.residue_intersection(&resolved, left, top, width, height)? {
                Some(product) => {
                    let (sx, sy) = self.pack_scratch(&product)?;
                    Some([sx as f32, sy as f32])
                }
                None => None,
            }
        } else {
            None
        };
        self.push_op(Op::Image(Box::new(ImageOp {
            image: image.0,
            inv: [
                inverse.a, inverse.b, inverse.c, inverse.d, inverse.e, inverse.f,
            ],
            image_rect: [bx0, by0, bx1, by1],
            dest: [left as f32, top as f32, vx1.ceil(), vy1.ceil()],
            clip: [
                resolved.rect.min.x,
                resolved.rect.min.y,
                resolved.rect.max.x,
                resolved.rect.max.y,
            ],
            residue_origin,
            axis_aligned: transform_preserves_axes(&to_device),
            alpha,
            linear: filter == ImageFilter::Linear,
            style: self.style,
            mask,
        })));
        Ok(())
    }

    /// The shading-space geometry of a non-solid paint. `None` means a singular
    /// shading transform made the sweep unmappable — a degenerate shading matrix
    /// paints nothing rather than something arbitrary (§4.7).
    ///
    /// Callers guarantee `paint` is not `Solid`. The shaded *command's* transform is
    /// deliberately absent here: a shading anchors to the scene through its own
    /// transform (§8.7.4.3), not to the path it fills.
    #[allow(clippy::cast_precision_loss)] // mesh anchors are device pixel indices
    fn shaded_geometry(&mut self, paint: Paint) -> Result<Option<ShadedGeometry>, RenderError> {
        match paint {
            // The two callers matched Solid off before calling.
            Paint::Solid(_) => unreachable!("shaded_geometry is called for non-solid paints only"),
            Paint::Shading {
                ramp,
                kind,
                transform,
            } => {
                if self.resources.ramp(ramp).is_none() {
                    return Err(RenderError::UnknownRamp { ramp });
                }
                self.used_ramps.insert(ramp.0);
                let Some(inverse) = transform.then(self.viewport.transform).invert() else {
                    return Ok(None);
                };
                let (kind_word, extend, geo0, geo1) = match kind {
                    ShadingKind::Axial { start, end, extend } => {
                        (0.0, extend, [start.x, start.y, end.x, end.y], [0.0; 4])
                    }
                    ShadingKind::Radial {
                        start,
                        start_radius,
                        end,
                        end_radius,
                        extend,
                    } => (
                        1.0,
                        extend,
                        [start.x, start.y, end.x, end.y],
                        [start_radius, end_radius, 0.0, 0.0],
                    ),
                };
                Ok(Some(ShadedGeometry {
                    paint: PaintSource::Ramp(ramp.0),
                    kind_word,
                    extend_bits: u32::from(extend.0) | (u32::from(extend.1) << 1),
                    geo0,
                    geo1,
                    inv: [
                        inverse.a, inverse.b, inverse.c, inverse.d, inverse.e, inverse.f,
                    ],
                }))
            }
            Paint::Mesh(mesh) => {
                let Some(stored) = self.resources.mesh(mesh) else {
                    return Err(RenderError::UnknownMesh { mesh });
                };
                self.used_meshes.insert(mesh.0);
                // Meshes sample at absolute device pixels (integration note 5): no
                // inverse needed, the anchor is the whole mapping.
                Ok(Some(ShadedGeometry {
                    paint: PaintSource::Mesh(mesh.0),
                    kind_word: 2.0,
                    extend_bits: 0,
                    geo0: [stored.spec.left as f32, stored.spec.top as f32, 0.0, 0.0],
                    geo1: [0.0; 4],
                    inv: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                }))
            }
        }
    }

    /// A shaded fill of a rect-hinted outline under an axis-preserving transform:
    /// analytic coverage, no scratch tile (the shading twin of ADR 0007's fast
    /// path).
    #[allow(clippy::cast_precision_loss)]
    fn push_shaded_rect(
        &mut self,
        geometry: ShadedGeometry,
        rect: Rect,
        to_device: &DeviceTransform,
        resolved: &ResolvedClip,
        style: DrawStyle,
        mask: Option<u32>,
    ) {
        let p0 = apply(to_device, rect.min);
        let p1 = apply(to_device, rect.max);
        let device_rect = Rect::new(
            Point::new(p0.x.min(p1.x), p0.y.min(p1.y)),
            Point::new(p0.x.max(p1.x), p0.y.max(p1.y)),
        );
        let vx0 = device_rect.min.x.max(resolved.rect.min.x).max(0.0);
        let vy0 = device_rect.min.y.max(resolved.rect.min.y).max(0.0);
        let vx1 = device_rect
            .max
            .x
            .min(resolved.rect.max.x)
            .min(self.viewport.width as f32);
        let vy1 = device_rect
            .max
            .y
            .min(resolved.rect.max.y)
            .min(self.viewport.height as f32);
        if vx0 >= vx1 || vy0 >= vy1 {
            return;
        }
        self.push_op(Op::Shaded(Box::new(ShadedOp {
            paint: geometry.paint,
            inv: geometry.inv,
            kind_word: geometry.kind_word,
            extend_bits: geometry.extend_bits,
            geo0: geometry.geo0,
            geo1: geometry.geo1,
            dest: [vx0.floor(), vy0.floor(), vx1.ceil(), vy1.ceil()],
            coverage_origin: None,
            coverage_rect: [
                device_rect.min.x,
                device_rect.min.y,
                device_rect.max.x,
                device_rect.max.y,
            ],
            clip: [
                resolved.rect.min.x,
                resolved.rect.min.y,
                resolved.rect.max.x,
                resolved.rect.max.y,
            ],
            style,
            mask,
        })));
    }

    /// A shaded fill or stroke through a rasterised coverage tile in scratch.
    #[allow(clippy::cast_precision_loss, clippy::arithmetic_side_effects)]
    fn push_shaded_coverage(
        &mut self,
        geometry: ShadedGeometry,
        polylines: &[Polyline],
        rule: Rule,
        resolved: &ResolvedClip,
        style: DrawStyle,
        mask: Option<u32>,
    ) -> Result<(), RenderError> {
        let Some(tile) = self.coverage_tile(polylines, rule, resolved)? else {
            return Ok(());
        };
        let (sx, sy) = self.pack_scratch(&tile)?;
        self.push_op(Op::Shaded(Box::new(ShadedOp {
            paint: geometry.paint,
            inv: geometry.inv,
            kind_word: geometry.kind_word,
            extend_bits: geometry.extend_bits,
            geo0: geometry.geo0,
            geo1: geometry.geo1,
            // The quad is exactly the tile: the shader's texel arithmetic
            // (`coverage.xy + p − dest.xy`) depends on it.
            dest: [
                tile.left as f32,
                tile.top as f32,
                (tile.left + tile.width.cast_signed()) as f32,
                (tile.top + tile.height.cast_signed()) as f32,
            ],
            coverage_origin: Some([sx as f32, sy as f32]),
            coverage_rect: [0.0; 4],
            clip: [
                resolved.rect.min.x,
                resolved.rect.min.y,
                resolved.rect.max.x,
                resolved.rect.max.y,
            ],
            style,
            mask,
        })));
        Ok(())
    }

    /// The glyph lane: rasterise (or find) the tile for this key and emit its quad.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    #[allow(clippy::arithmetic_side_effects, clippy::cast_precision_loss)]
    #[allow(clippy::too_many_arguments)] // one draw's parameters, threaded once
    #[allow(clippy::too_many_arguments)] // one draw's parameters, threaded once
    fn push_glyph(
        &mut self,
        outline: OutlineId,
        to_device: &DeviceTransform,
        placement: &GlyphPlacement,
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

        let entry = if let Some(entry) = self.atlas.get(&key) {
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

    fn push_rect_instance(&mut self, rect: Rect, color: quorra_scene::Color, mask: Option<u32>) {
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
        let style = self.style;
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

    fn push_op(&mut self, op: Op) {
        self.plan_mut().ops.push(op);
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
    use quorra_scene::{Affine, Color, Point, Rect, SceneBuilder};

    use super::{BatchKind, ScratchPacker, encode};
    use crate::atlas::AtlasStore;
    use crate::error::RenderError;
    use crate::resources::ResourceStore;
    use crate::startup::Coverage;
    use crate::viewport::Viewport;

    /// **A shelf no CPU tile wrote is blank sheet, not the tail of the wide layout.**
    ///
    /// The packer lays rows out at the *packing* width and `finish` restrides them down
    /// to the width the shelves reached (ADR 0021). Compaction moves each row left and
    /// leaves the old bytes behind it, so growing the buffer to the sheet's extent
    /// straight afterwards keeps whatever of that tail falls inside — stale coverage,
    /// not blank sheet.
    ///
    /// Invisible while every shelf holds CPU tiles, because each writes its own rows.
    /// The GPU lane reserves rows it fills on the device, so its shelf is exactly the
    /// region that reads back whatever was left there: the caller's
    /// `transparency_group.pdf` drew 136 410 texels of another shape's coverage in
    /// horizontal streaks under its last CPU tile (`QUORRA_FEEDBACK.md` §20.4.1).
    #[test]
    fn a_shelf_the_cpu_lane_did_not_write_is_blank() {
        let mut packer = ScratchPacker::new(64, 64);
        let mask = |width: u32, height: u32| crate::raster::CoverageMask {
            left: 0,
            top: 0,
            width,
            height,
            coverage: vec![255; (width * height) as usize],
        };
        // Two tiles on one shelf: 16 of the 64 packing columns are used, so `finish`
        // restrides — which is the precondition for the tail to exist at all.
        assert!(packer.pack(&mask(8, 20)).is_some());
        assert!(packer.pack(&mask(8, 20)).is_some());
        // And a shelf below them that reserves rows without writing bytes, which is
        // what every GPU-lane tile does. Short enough that the shelf rule opens a new
        // one for it rather than seating it beside the two above.
        assert_eq!(packer.reserve(8, 4), Some((0, 20)));

        let scratch = packer.finish().expect("the sheet holds three tiles");
        assert_eq!((scratch.width, scratch.height), (16, 24));
        let stray = scratch.data[(16 * 20) as usize..]
            .iter()
            .filter(|byte| **byte != 0)
            .count();
        assert_eq!(stray, 0, "{stray} texels of somebody else's coverage");
    }

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
