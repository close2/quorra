//! A clip chain, resolved: the rectangular part, which costs a pixel nothing, and the
//! residue, which has to become coverage.
//!
//! ISO 32000-2 §8.5.4 makes a chain one region arrived at by intersection, and this
//! module holds the whole of that reading. A link whose outline is a rectangle under
//! its own transform intersects into the resolved rectangle and is applied by geometry
//! alone (ADR 0007); a link that is anything else is kept as a residue and rasterised
//! into one coverage tile at draw time, where the links intersect rather than multiply
//! (ADR 0030). Chains are memoised across shared prefixes, because a page shares them:
//! the caller's worst holds 3 608.
//!
//! The residue chain's shape is this module's alone — [`Encoder::residue_intersection`]
//! is the only reader of a link, which is why it sits here rather than beside the
//! rasterising lanes it hands its tile to.

use std::sync::Arc;

use quorra_scene::{ClipId, FillRule, Point, Rect, Scene};

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
    pub(super) fn residue_intersection(
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
}
