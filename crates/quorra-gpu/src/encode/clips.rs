//! A clip chain, resolved: the rectangular part, which costs a pixel nothing, and the
//! residue, which has to become coverage.
//!
//! ISO 32000-2 §8.5.4 makes a chain one region arrived at by intersection, and this
//! module holds the whole of that reading. A link whose outline is a rectangle under
//! its own transform intersects into the resolved rectangle and is applied by geometry
//! alone (ADR 0007); a link that is anything else is kept as a residue and rasterised at
//! draw time, where the links intersect rather than multiply (ADR 0030) and where the
//! chain covers the region it occupies rather than the mark that asked (ADR 0049).
//! Chains are memoised across shared prefixes, because a page shares them: the caller's
//! worst holds 3 608.
//!
//! The residue chain's shape is this module's alone — [`Encoder::residue_intersection`]
//! is the only reader of a link, which is why it sits here rather than beside the
//! rasterising lanes it hands its tile to. **Whether a region is worth keeping is not
//! this module's question**: that is [`super::residue`], which knows what the scene will
//! ask for and what the frame may spend, and knows nothing about a link.

use std::sync::Arc;

use quorra_scene::{ClipId, FillRule, Point, Rect, Scene};

use super::residue::Verdict;
use super::{Encoder, apply, compose, transform_preserves_axes};
use crate::error::RenderError;
use crate::raster::{self, Rule};
use crate::resources::ResourceStore;
use crate::viewport::Viewport;

/// A clip rectangle that admits everything, for unclipped instances.
pub(super) const OPEN_CLIP: [f32; 4] = [-1.0e9, -1.0e9, 1.0e9, 1.0e9];

/// A resolved clip chain: the intersection of its rectangular links, plus the chain
/// of non-rectangular links (the residue) that must multiply into a coverage mask.
#[derive(Debug, Clone)]
pub(super) struct ResolvedClip {
    pub(super) rect: Rect,
    pub(super) residues: Option<Arc<ResidueLink>>,
}

#[derive(Debug)]
pub(super) struct ResidueLink {
    clip: ClipId,
    parent: Option<Arc<ResidueLink>>,
}

pub(super) fn open_clip() -> ResolvedClip {
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
pub(super) struct ClipResolver {
    pub(super) resolved: Vec<Option<ResolvedClip>>,
}

impl ClipResolver {
    pub(super) fn new(clip_count: usize) -> Self {
        Self {
            resolved: vec![None; clip_count],
        }
    }

    /// Iterative on purpose: chains are deep on real pages and a recursive walk
    /// would put the depth on the stack. Cycles cannot occur — a parent id is always
    /// smaller than its child's, by construction in `SceneBuilder::clip`.
    pub(super) fn resolve(
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

impl Encoder<'_> {
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
    ///
    /// **The chain is rasterised over its own region and this is a window on it**
    /// wherever that region is worth keeping (ADR 0049) — which is the same reading one
    /// step further on, since a region that does not depend on the mark asking about it
    /// has no business being rasterised once per mark. [`super::residue::ResidueRegions`] holds
    /// the rule; what is left here is the chain, which is this module's own.
    pub(super) fn residue_intersection(
        &mut self,
        resolved: &ResolvedClip,
        left: i32,
        top: i32,
        width: u32,
        height: u32,
    ) -> Result<Option<raster::CoverageMask>, RenderError> {
        let Some(leaf) = resolved.residues.clone() else {
            return Ok(None);
        };
        let key = leaf.clip.0;
        if let Verdict::Region(region) = self.residue.verdict(key) {
            let span = self.clock.start();
            let tile = window(region, left, top, width, height);
            self.clock.geometry(span);
            return Ok(Some(tile));
        }
        let links = self.flatten_chain(&leaf)?;
        if matches!(self.residue.verdict(key), Verdict::Undecided) {
            let region = chain_region(&links, self.visible);
            let region_bytes = region.map_or(0, |(_, _, w, h)| area(w, h));
            let tile_bytes = area(width, height);
            if self.residue.admit(key, region_bytes, tile_bytes) {
                let mask = region.map(|(l, t, w, h)| self.intersect_links(&links, l, t, w, h));
                let tile = window(mask.as_ref(), left, top, width, height);
                self.residue.insert(key, mask);
                return Ok(Some(tile));
            }
        }
        self.residue.note_tile();
        Ok(Some(self.intersect_links(&links, left, top, width, height)))
    }

    /// Every link of a chain, flattened into device space once.
    ///
    /// The whole chain at once rather than a link at a time, because both callers below
    /// rasterise every link and the region path would otherwise flatten them again. What
    /// is held is bounded by the outlines the chain names, which were budgeted when the
    /// caller uploaded them.
    fn flatten_chain(&self, leaf: &Arc<ResidueLink>) -> Result<Vec<FlatLink>, RenderError> {
        let mut links = Vec::new();
        let mut residue = Some(Arc::clone(leaf));
        while let Some(link) = residue.take() {
            let def = &self.scene.clips()[link.clip.0 as usize];
            let stored =
                self.resources
                    .outline(def.outline)
                    .ok_or(RenderError::UnknownOutline {
                        outline: def.outline,
                    })?;
            let span = self.clock.start();
            let polylines =
                raster::flatten(&stored.segments, compose(def.transform, self.viewport));
            self.clock.geometry(span);
            links.push(FlatLink {
                polylines,
                rule: match def.rule {
                    FillRule::NonZero => Rule::NonZero,
                    FillRule::EvenOdd => Rule::EvenOdd,
                },
            });
            residue.clone_from(&link.parent);
        }
        Ok(links)
    }

    /// The chain's links rasterised over one rectangle and intersected — `min`, per
    /// pixel, which is the clause's intersection and this function's whole subject.
    fn intersect_links(
        &self,
        links: &[FlatLink],
        left: i32,
        top: i32,
        width: u32,
        height: u32,
    ) -> raster::CoverageMask {
        let span = self.clock.start();
        let mut combined: Option<raster::CoverageMask> = None;
        for link in links {
            let mask = raster::fill_mask(&link.polylines, link.rule, left, top, width, height);
            combined = Some(match combined {
                None => mask,
                Some(mut base) => {
                    for (m, l) in base.coverage.iter_mut().zip(&mask.coverage) {
                        *m = (*m).min(*l);
                    }
                    base
                }
            });
        }
        self.clock.geometry(span);
        // A chain with no link never reaches here: `residue_intersection` returns before
        // it, and `flatten_chain` is only called on a chain that has a leaf.
        combined.unwrap_or_else(|| raster::CoverageMask::transparent(left, top, width, height))
    }
}

/// The bytes a coverage mask of these dimensions holds. Saturating because both extents
/// are scene-derived: a product that could not fit is one the budget below refuses.
fn area(width: u32, height: u32) -> u64 {
    u64::from(width).saturating_mul(u64::from(height))
}

/// One residue link, flattened into device space.
struct FlatLink {
    polylines: Vec<raster::Polyline>,
    rule: Rule,
}

/// A region's window over one tile, or a transparent tile when the chain's links leave
/// no region at all.
fn window(
    region: Option<&raster::CoverageMask>,
    left: i32,
    top: i32,
    width: u32,
    height: u32,
) -> raster::CoverageMask {
    region.map_or_else(
        || raster::CoverageMask::transparent(left, top, width, height),
        |region| region.crop(left, top, width, height),
    )
}

/// The device pixels a chain can mark: its links' bounds intersected, held to the
/// target, rounded out to whole pixels. `None` when that leaves nothing — which is a
/// region that admits nothing, not a missing one.
///
/// Intersected because the links intersect (ADR 0030), and outside any one link's own
/// bounds a closed path winds nothing: the chain is transparent there whatever the
/// others say.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[allow(clippy::arithmetic_side_effects, clippy::cast_precision_loss)]
fn chain_region(links: &[FlatLink], target: Rect) -> Option<(i32, i32, u32, u32)> {
    let mut region = target;
    for link in links {
        let (x0, y0, x1, y1) = raster::polyline_bounds(&link.polylines)?;
        region = region.intersection(Rect::new(Point::new(x0, y0), Point::new(x1, y1)));
    }
    if region.is_empty() {
        return None;
    }
    let left = region.min.x.floor() as i32;
    let top = region.min.y.floor() as i32;
    let width = (region.max.x.ceil() as i32 - left).max(0) as u32;
    let height = (region.max.y.ceil() as i32 - top).max(0) as u32;
    (width > 0 && height > 0).then_some((left, top, width, height))
}
