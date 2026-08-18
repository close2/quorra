//! The analytic rectangle lane, and the one command that is a rectangle by name.
//!
//! ADR 0007's lane: a rectangle intersected with a rectangle is a rectangle, so an
//! axis-aligned mark under an axis-preserving transform inside a rectangular clip needs
//! no coverage at all — the clip is applied here by intersection, the shader evaluates
//! one box, and a rectangular clip costs a pixel nothing. That is the premise the brief's
//! §0 names beside repeated glyphs, and it is why the box is computed in exactly one
//! place: [`Encoder::clipped_device_rect`] is shared by the two commands that reach the
//! lane — a `Command::Rect` and, since ADR 0047, a solid fill whose outline is four
//! axis-aligned edges, which is the only form a document's rectangles actually arrive
//! in. Two arms that computed the box separately would be two arms that could come to
//! draw different marks for one rectangle.
//!
//! When any of the lane's conditions fails, the same rectangle is four corners and takes
//! the path lane, exactly.

use quorra_scene::{Affine, ClipId, MaskId, Point, Rect};

use super::Encoder;
use super::clips::ResolvedClip;
use super::device_space::{apply, compose, transform_preserves_axes};
use crate::error::RenderError;
use crate::raster::{DeviceTransform, Polyline, Rule};

impl Encoder<'_> {
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
    ///
    /// [`target_rect`]: super::device_space::target_rect
    /// [`CULL_MARGIN`]: super::device_space::CULL_MARGIN
    pub(super) fn clipped_device_rect(
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
    pub(super) fn encode_rect(
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
            self.push_rect_instance(device_rect, color, self.style, mask)?;
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
        // A fill, not a stroke: the four corners' device box is the only bound on how
        // thin this parallelogram is, which is the residual ADR 0070 states.
        self.push_coverage(&polylines, Rule::NonZero, color, &resolved, None, mask)
    }
}
