//! What one walk hands back, and what the walk cost.
//!
//! [`Encoded`] is a frame's whole encoding: the two instance streams, the coverage
//! sheet, the layer tree, the resources the device must realise before a pass runs, and
//! the counters `Counters` publishes. [`finish`] is the other half of the same subject
//! and is why the two are one module — the walk decides what a page draws, and this
//! prices what the walk placed: the sheet's extent, which is only known once every tile
//! has a shelf, the GPU lane's own bytes, and the counts read out of the encoder's
//! working state.
//!
//! Nothing here decides anything about a mark. Every number is either a buffer the walk
//! filled or an exact function of one.

use std::collections::HashSet;

use super::clips::ClipResolver;
use super::{
    Encoder, FunctionOp, ImageOp, LayerPlan, MaskPlan, Op, Scratch, ScratchPacker, ShadedOp,
};
use crate::error::RenderError;
use crate::frame::LaneCounts;
use crate::instrument::EncodeClock;

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
    /// Reduced image variants this frame draws, `(image, x factor, y factor)`, sorted
    /// (ADR 0089).
    pub used_reductions: Vec<(u32, u32, u32)>,
    pub used_ramps: Vec<u32>,
    pub used_meshes: Vec<u32>,
    /// The raw ids of every §7.10.5 program this frame paints with. Nothing has to be
    /// realised for one — the pipeline is compiled on first use, keyed by content — but
    /// the frame owes a `Report` for each program that reads an empty operand stack, and
    /// this is the list that says which (ADR 0053).
    pub used_functions: Vec<u32>,
    pub commands: u32,
    /// Which lane made each mark's coverage (§1.1).
    pub lanes: LaneCounts,
    /// Coverage tiles this frame placed on the scratch sheet, both lanes.
    pub tiles: u32,
    /// The same tiles priced: the sheet's extent and the texels on it (ADR 0057).
    pub coverage: crate::frame::CoverageSheet,
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
    /// The compute lane's edges and tiles for this frame; empty except under
    /// `Coverage::Compute` (ADR 0080).
    pub compute: crate::compute::ComputeSheet,
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
    /// The walk's viewport-free half, when every structure this frame met can be
    /// replayed per record at another viewport (`replay.rs`, ADR 0087) — `None` for a
    /// frame with a child layer, a soft mask, a residue clip, an atlas or winding
    /// tile, and for every lane but [`Coverage::Compute`](crate::startup::Coverage).
    pub replay: Option<super::ReplayList>,
    /// Glyph-lane marks this frame drew through the scratch sheet because the packer had
    /// no room — see `Counters::atlas_overflow_tiles`, which reports it.
    pub atlas_overflow_tiles: u32,
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
                    Op::Function(_) => total.saturating_add(size_of::<FunctionOp>() as u64),
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
            self.replay
                .as_ref()
                .map_or(0, super::ReplayList::retained_bytes),
            scratch,
            bytes_of(self.winding.vertices.len(), size_of::<f32>()),
            bytes_of(self.winding.tiles.len(), size_of::<crate::pane::Tile>()),
            self.compute.retained_bytes(),
            plan_bytes(&self.root),
            self.layers.iter().map(plan_bytes).sum::<u64>(),
            bytes_of(self.layers.len(), size_of::<LayerPlan>()),
            bytes_of(self.mask_plans.len(), size_of::<Option<MaskPlan>>()),
            bytes_of(self.used_images.len(), size_of::<u32>()),
            bytes_of(self.used_reductions.len(), size_of::<(u32, u32, u32)>()),
            bytes_of(self.used_ramps.len(), size_of::<u32>()),
            bytes_of(self.used_meshes.len(), size_of::<u32>()),
            bytes_of(self.used_functions.len(), size_of::<u32>()),
        ]
        .into_iter()
        .fold(0_u64, u64::saturating_add)
    }
}

/// What the frame is worth once every tile has been placed: the sheet's own extent, the
/// GPU lane's, and the counters the walk accumulated.
///
/// Split from [`encode`](super::encode) because it is a different subject — the walk decides what a
/// page draws, this prices what the walk placed — and because the two together read as
/// one function only to someone who has already read both.
pub(super) fn finish(mut encoder: Encoder<'_>, commands: usize) -> Result<Encoded, RenderError> {
    // The sheet's extent is only known once every tile has been placed, so the GPU
    // lane learns it here rather than carrying a guess: its triangles are already in
    // sheet coordinates, and what was missing was how large the sheet turned out to
    // be. Then the lane's own cost is charged — scene-derived arithmetic, priced where
    // nothing has been allocated yet, against the same one number (principle 3). A
    // frame whose sheet holds no GPU tiles is charged nothing here, because it
    // allocates nothing there: `Sheet::device_bytes` states that condition once.
    let mut winding = std::mem::take(&mut encoder.winding);
    let mut compute = std::mem::take(&mut encoder.compute);
    let packer = std::mem::replace(&mut encoder.scratch, ScratchPacker::new(1, 1));
    let tiles = packer.placed;
    let coverage = packer.state();
    let scratch = packer.finish();
    if let Some(sheet) = scratch.as_ref() {
        winding.width = sheet.width;
        winding.height = sheet.height;
        compute.width = sheet.width;
        compute.height = sheet.height;
        // The sheet is one texture, and until ADR 0021 the only thing charged for it
        // was the area of the tiles *on* it — so the largest scene-derived allocation
        // a page of path work makes was the one number nobody counted, which is the
        // reverse of what principle 3 asks. Shelf packing leaves gaps, and the gaps
        // are allocated too: charge the difference, once, now that the extent is known.
        let sheet_bytes = u64::from(sheet.width).saturating_mul(u64::from(sheet.height));
        encoder.charge(sheet_bytes.saturating_sub(encoder.scratch_charged))?;
    }
    encoder.charge(winding.device_bytes())?;
    // The compute lane's per-tile bytes were charged at each commit; what only the
    // final extent decides is the image buffer, one aligned row stride tall the sheet's
    // height (ADR 0080).
    if !compute.is_empty() {
        let stride = u64::from(compute.width).div_ceil(256).saturating_mul(256);
        encoder.charge(stride.saturating_mul(u64::from(compute.height)))?;
    }

    let sorted = |set: HashSet<u32>| {
        let mut ids: Vec<u32> = set.into_iter().collect();
        ids.sort_unstable();
        ids
    };

    // The list carries the two counters the replay cannot recount, so the encode it
    // produces reports what the walk reported.
    let replay = encoder.replay.take().map(|mut list| {
        let (distinct, segments) = encoder.seeded_counts.unwrap_or((
            u32::try_from(encoder.distinct_outlines.len()).unwrap_or(u32::MAX),
            u32::try_from(encoder.segments).unwrap_or(u32::MAX),
        ));
        list.distinct_outlines = distinct;
        list.segments = segments;
        list
    });
    Ok(Encoded {
        replay,
        rect_instances: encoder.rect_instances,
        quad_instances: encoder.quad_instances,
        root: encoder.root,
        layers: encoder.layers,
        mask_plans: encoder.mask_plans,
        scratch,
        used_images: sorted(encoder.used_images),
        used_reductions: {
            let mut ids: Vec<(u32, u32, u32)> = encoder.used_reductions.into_iter().collect();
            ids.sort_unstable();
            ids
        },
        used_ramps: sorted(encoder.used_ramps),
        used_meshes: sorted(encoder.used_meshes),
        used_functions: sorted(encoder.used_functions),
        encode_phases: encoder.clock,
        lanes: encoder.lanes,
        tiles,
        coverage,
        commands: u32::try_from(commands).unwrap_or(u32::MAX),
        clip_distinct_regions: distinct_clip_regions(&encoder.clips),
        clip_residue_regions: encoder.residue.regions,
        clip_residue_tiles: encoder.residue.tiles,
        distinct_outlines: encoder.seeded_counts.map_or_else(
            || u32::try_from(encoder.distinct_outlines.len()).unwrap_or(u32::MAX),
            |(distinct, _)| distinct,
        ),
        atlas_distinct_keys: u32::try_from(encoder.atlas_keys.len()).unwrap_or(u32::MAX),
        segments: encoder.seeded_counts.map_or_else(
            || u32::try_from(encoder.segments).unwrap_or(u32::MAX),
            |(_, segments)| segments,
        ),
        commands_culled: encoder.culled,
        layers_culled: encoder.culled_layers,
        winding,
        compute,
        atlas_pressure: encoder.atlas_pressure,
        atlas_requested_bytes: encoder.atlas_requested_bytes,
        atlas_entries_used: encoder.atlas_entries_used,
        atlas_overflow_tiles: encoder.atlas_overflow_tiles,
    })
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
