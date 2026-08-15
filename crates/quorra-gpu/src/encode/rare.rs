//! The rare-case lanes: an image, a ramp sweep, a mesh, a §7.10.5 program — one
//! uniform-driven quad each.
//!
//! The brief's §0 premise is that most of a page is a few glyph outlines repeated and
//! axis-aligned rectangles; ADR 0011 encodes what is left to match that premise rather
//! than to match its own complexity. An image (ISO 32000-2 §8.9.5) and a shading
//! (§8.7.4.5) each become a single quad carrying its own parameters, not a third and
//! fourth instance stream whose plumbing no measured page would fill.
//!
//! The two lanes are one module because they answer in the same shape: the fragment
//! shader maps device pixels back through an inverse transform carried in the op, so
//! the quad only has to *cover* the footprint, and the coverage it is weighted by is
//! either analytic — an axis-preserving image placement, a rect-hinted outline — or one
//! tile of the frame's scratch sheet, rasterised by the same CPU rasteriser every other
//! lane's coverage comes from.
//!
//! What the ops mean to the device that draws them is `device.rs`'s half; what a
//! command means to them is here.
//!
//! # The seam between a mark and its paint
//!
//! [`QuadPlacement`] is *where* a quad goes and what weights it; a paint is *what colour*
//! it is. The four lanes above and the function lane of ADR 0053 differ only in the second,
//! so the first is computed once, in [`Encoder::rect_placement`] and
//! [`Encoder::coverage_placement`], and the two ops are built from it. That seam is the
//! reason a device-evaluated colour needed no second copy of the tile arithmetic — the part
//! where "the quad is exactly the tile" is load-bearing for the shader's texel lookup.

use quorra_scene::{
    Affine, BlendMode, ClipId, ImageFilter, ImageId, MaskId, Paint, Point, Rect, ShadingKind,
};

use super::clips::ResolvedClip;
use super::function::FunctionGeometry;
use super::{ChildOp, DrawStyle, Encoder, Op, apply, compose, transform_preserves_axes};
use crate::error::RenderError;
use crate::raster::{DeviceTransform, Polyline, Rule};

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
pub(super) struct ShadedGeometry {
    paint: PaintSource,
    kind_word: f32,
    extend_bits: u32,
    geo0: [f32; 4],
    geo1: [f32; 4],
    inv: [f32; 6],
}

/// Where a rare-case quad goes, what weights it, and under which of ADR 0010's styles.
///
/// The half of a quad op that does not depend on the paint. Both lanes that draw one build
/// it through the same two functions, so "the quad is exactly the tile" — which the
/// shaders' texel arithmetic (`coverage.xy + p − dest.xy`) depends on — is one statement
/// rather than one per lane.
#[derive(Debug, Clone, Copy)]
pub(crate) struct QuadPlacement {
    /// The quad drawn; when coverage comes from scratch, exactly the tile's bounds.
    pub dest: [f32; 4],
    /// The coverage tile's origin in scratch, or `None` for the analytic rectangle.
    pub coverage_origin: Option<[f32; 2]>,
    /// The analytic coverage rectangle (the shape itself), used when `coverage_origin`
    /// is `None`.
    pub coverage_rect: [f32; 4],
    pub clip: [f32; 4],
    pub style: DrawStyle,
    pub mask: Option<u32>,
}

/// A paint that draws as one quad: the ramp sweeps and meshes of the shading lane, or a
/// §7.10.5 program the device evaluates (ADR 0053).
///
/// One enum rather than two paths through the fill and stroke walks, so that `encode.rs`
/// resolves a paint and places a quad without knowing which of the two it has — the
/// difference is entirely in what is bound at draw time.
#[derive(Debug, Clone, Copy)]
pub(super) enum RarePaint {
    Shaded(ShadedGeometry),
    Function(FunctionGeometry),
}

impl Encoder<'_> {
    /// The image arm (ISO 32000-2 §8.9.5): one uniform-driven quad per placement,
    /// with a non-Normal blend through an implicit child, as fills take it.
    #[allow(clippy::too_many_arguments)] // one command's fields, destructured once
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    #[allow(clippy::arithmetic_side_effects, clippy::cast_precision_loss)]
    pub(super) fn encode_image(
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
            self.push_op(Op::Child(ChildOp::implicit_blend_group(child, blend, mask)));
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
    pub(super) fn rare_paint(&mut self, paint: Paint) -> Result<Option<RarePaint>, RenderError> {
        match paint {
            // The two callers matched Solid off before calling.
            Paint::Solid(_) => unreachable!("rare_paint is called for non-solid paints only"),
            Paint::Function { .. } => Ok(self.function_geometry(paint)?.map(RarePaint::Function)),
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
                Ok(Some(RarePaint::Shaded(ShadedGeometry {
                    paint: PaintSource::Ramp(ramp.0),
                    kind_word,
                    extend_bits: u32::from(extend.0) | (u32::from(extend.1) << 1),
                    geo0,
                    geo1,
                    inv: [
                        inverse.a, inverse.b, inverse.c, inverse.d, inverse.e, inverse.f,
                    ],
                })))
            }
            Paint::Mesh(mesh) => {
                let Some(stored) = self.resources.mesh(mesh) else {
                    return Err(RenderError::UnknownMesh { mesh });
                };
                self.used_meshes.insert(mesh.0);
                // Meshes sample at absolute device pixels (integration note 5): no
                // inverse needed, the anchor is the whole mapping.
                Ok(Some(RarePaint::Shaded(ShadedGeometry {
                    paint: PaintSource::Mesh(mesh.0),
                    kind_word: 2.0,
                    extend_bits: 0,
                    geo0: [stored.spec.left as f32, stored.spec.top as f32, 0.0, 0.0],
                    geo1: [0.0; 4],
                    inv: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                })))
            }
        }
    }

    /// A rare-case fill of a rect-hinted outline under an axis-preserving transform:
    /// analytic coverage, no scratch tile (the shading twin of ADR 0007's fast
    /// path).
    pub(super) fn push_rare_rect(
        &mut self,
        paint: RarePaint,
        rect: Rect,
        to_device: &DeviceTransform,
        resolved: &ResolvedClip,
        style: DrawStyle,
        mask: Option<u32>,
    ) {
        let Some(placement) = self.rect_placement(rect, to_device, resolved, style, mask) else {
            return;
        };
        self.push_op(paint.at(placement));
    }

    /// A rare-case fill or stroke through a rasterised coverage tile in scratch.
    pub(super) fn push_rare_coverage(
        &mut self,
        paint: RarePaint,
        polylines: &[Polyline],
        rule: Rule,
        resolved: &ResolvedClip,
        style: DrawStyle,
        mask: Option<u32>,
    ) -> Result<(), RenderError> {
        let Some(placement) = self.coverage_placement(polylines, rule, resolved, style, mask)?
        else {
            return Ok(());
        };
        self.push_op(paint.at(placement));
        Ok(())
    }

    /// Where the quad goes for a rect-hinted shape: the shape's device rectangle, cut to
    /// the clip and the target and expanded to pixel bounds, with the shape itself as the
    /// analytic coverage. `None` is a mark that reaches no pixel.
    #[allow(clippy::cast_precision_loss)] // target sizes are far below 2^24
    fn rect_placement(
        &mut self,
        rect: Rect,
        to_device: &DeviceTransform,
        resolved: &ResolvedClip,
        style: DrawStyle,
        mask: Option<u32>,
    ) -> Option<QuadPlacement> {
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
            return None;
        }
        Some(QuadPlacement {
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
        })
    }

    /// Where the quad goes for a rasterised shape: exactly the coverage tile, which is
    /// what both shaders' texel arithmetic (`coverage.xy + p − dest.xy`) depends on.
    #[allow(clippy::cast_precision_loss, clippy::arithmetic_side_effects)]
    fn coverage_placement(
        &mut self,
        polylines: &[Polyline],
        rule: Rule,
        resolved: &ResolvedClip,
        style: DrawStyle,
        mask: Option<u32>,
    ) -> Result<Option<QuadPlacement>, RenderError> {
        let Some(tile) = self.coverage_tile(polylines, rule, resolved)? else {
            return Ok(None);
        };
        let (sx, sy) = self.pack_scratch(&tile)?;
        Ok(Some(QuadPlacement {
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
        }))
    }
}

impl RarePaint {
    /// The op this paint becomes once the mark's placement is known.
    fn at(self, placement: QuadPlacement) -> Op {
        match self {
            Self::Shaded(geometry) => Op::Shaded(Box::new(ShadedOp {
                paint: geometry.paint,
                inv: geometry.inv,
                kind_word: geometry.kind_word,
                extend_bits: geometry.extend_bits,
                geo0: geometry.geo0,
                geo1: geometry.geo1,
                dest: placement.dest,
                coverage_origin: placement.coverage_origin,
                coverage_rect: placement.coverage_rect,
                clip: placement.clip,
                style: placement.style,
                mask: placement.mask,
            })),
            Self::Function(geometry) => Op::Function(Box::new(geometry.at(placement))),
        }
    }
}
