//! The vocabulary as a caller reaches it: one method per [`Command`], each of them the
//! same three steps in the same order.
//!
//! Check the arguments (`validate`), push the command into the innermost open frame
//! (`frames`), return. Nothing else happens here, and that is the point — a method that
//! did more would be a method whose refusals a reader has to go looking for. The two
//! methods that take a closure, [`SceneBuilder::group`] and [`SceneBuilder::mask`], are
//! the same shape with a frame around the body.
//!
//! The signatures are the brief's §2.3, argument for argument, with the one divergence
//! `doc/PLAN.md` integration note 8 records: the `mask` parameter comes last.

use std::sync::Arc;

use super::frames::OpenFrame;
use super::{ClipDef, Command, GroupSpec, MaskDef, Scene, SceneData, cost};
use crate::blend::{BlendMode, Compose, FillRule};
use crate::error::SceneError;
use crate::geom::{Affine, Rect};
use crate::ids::ClipId;
use crate::ids::{ImageId, MaskId, OutlineId};
use crate::mask::{MaskKind, Transfer};
use crate::paint::{Color, Paint, Stroke};
use crate::scene::ImageFilter;

/// Builds a [`Scene`], validating every input at this boundary (§4.7).
///
/// Requires no device, runs on any thread — the caller's interpreter builds scenes on a
/// worker thread while the GPU is still initialising (§2.3). Resource identifiers
/// ([`OutlineId`] and friends) are opaque here: they belong to a device, and the device
/// validates them against its own registry at render time.
///
/// The four fields are visible to the rest of this module and to nothing else:
/// `frames` owns the stack and where a command lands, `validate` answers "is this
/// identifier one this scene allocated?" from the two counts, and both need the state
/// rather than a copy of it.
#[derive(Debug, Default)]
pub struct SceneBuilder {
    /// Finished top-level commands.
    pub(super) commands: Vec<Command>,
    /// One frame per open group or mask body; commands land in the innermost frame.
    pub(super) open_frames: Vec<OpenFrame>,
    pub(super) clips: Vec<ClipDef>,
    pub(super) masks: Vec<MaskDef>,
}

impl SceneBuilder {
    /// An empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an axis-aligned rectangle filled with a solid colour.
    ///
    /// An *empty* rectangle (zero width or height) is accepted and draws nothing, the
    /// same way a blank scene is legitimate.
    ///
    /// # Errors
    ///
    /// Refuses, without appending, any input §4.7 forbids: non-finite or unordered
    /// rectangles, non-finite transforms, coordinates or coefficients beyond
    /// [`MAX_COORDINATE`](super::MAX_COORDINATE), colours outside their range, and clip
    /// identifiers this scene never allocated. The error names the value and the limit;
    /// nothing is clamped or repaired.
    pub fn rect(
        &mut self,
        rect: Rect,
        transform: Affine,
        color: Color,
        clip: Option<ClipId>,
        mask: Option<MaskId>,
    ) -> Result<(), SceneError> {
        Self::check_rect(rect)?;
        Self::check_transform(transform)?;
        Self::check_color(color)?;
        self.check_clip(clip)?;
        self.check_mask(mask)?;
        self.push(Command::Rect {
            rect,
            transform,
            color,
            clip,
            mask,
        });
        Ok(())
    }

    /// Append a fill of an uploaded outline (§2.3 of the brief). The soft-mask
    /// parameter of the brief's signature arrives with M6, which owns masks entirely.
    ///
    /// # Errors
    ///
    /// Refuses invalid transforms, paints, and clip identifiers this scene never
    /// allocated. The outline identifier is opaque here; the device validates it
    /// against its registry at render time.
    // The brief's §2.3 signature, kept argument-for-argument so the caller's encoder
    // stays mechanical (one command, one call, no packing step). When M6 adds the
    // mask parameter this is revisited against a params struct, with the caller.
    #[allow(clippy::too_many_arguments)]
    pub fn fill(
        &mut self,
        outline: OutlineId,
        transform: Affine,
        rule: FillRule,
        paint: Paint,
        clip: Option<ClipId>,
        blend: BlendMode,
        compose: Compose,
        mask: Option<MaskId>,
    ) -> Result<(), SceneError> {
        Self::check_transform(transform)?;
        Self::check_paint(paint)?;
        self.check_clip(clip)?;
        self.check_mask(mask)?;
        Self::check_staged_compose(compose, blend)?;
        self.push(Command::Fill {
            outline,
            transform,
            rule,
            paint,
            clip,
            blend,
            compose,
            mask,
        });
        Ok(())
    }

    /// Append a stroke of an uploaded outline (§2.3 of the brief; parameters resolved
    /// upstream per §4.5). The soft-mask parameter arrives with M6.
    ///
    /// # Errors
    ///
    /// Refuses invalid transforms, strokes ([`Stroke::is_valid`]), paints, and clip
    /// identifiers this scene never allocated.
    // The brief's §2.3 signature (see the `fill` note; integration note 8 records
    // the mask-last divergence).
    #[allow(clippy::too_many_arguments)]
    pub fn stroke(
        &mut self,
        outline: OutlineId,
        transform: Affine,
        stroke: Stroke,
        paint: Paint,
        clip: Option<ClipId>,
        blend: BlendMode,
        mask: Option<MaskId>,
    ) -> Result<(), SceneError> {
        Self::check_transform(transform)?;
        Self::check_stroke(stroke)?;
        Self::check_paint(paint)?;
        self.check_clip(clip)?;
        self.check_mask(mask)?;
        self.push(Command::Stroke {
            outline,
            transform,
            stroke,
            paint,
            clip,
            blend,
            mask,
        });
        Ok(())
    }

    /// Allocate a clip region: an outline under a transform, admitting points by
    /// `rule`, intersected with `parent` when given (a chain is an intersection,
    /// §4.7). Returns the identifier commands reference it by — scene-scoped, meaning
    /// nothing to any other scene.
    ///
    /// # Errors
    ///
    /// Refuses invalid transforms and a `parent` this scene never allocated.
    pub fn clip(
        &mut self,
        outline: OutlineId,
        transform: Affine,
        rule: FillRule,
        parent: Option<ClipId>,
    ) -> Result<ClipId, SceneError> {
        Self::check_transform(transform)?;
        self.check_clip(parent)?;
        // The count fits u32 or the scene has bigger problems; the check keeps the
        // conversion honest rather than expecting it.
        let id = u32::try_from(self.clips.len()).map_err(|_| SceneError::UnknownClip {
            clip: ClipId(u32::MAX),
            allocated: u32::MAX,
        })?;
        self.clips.push(ClipDef {
            outline,
            transform,
            rule,
            parent,
        });
        Ok(ClipId(id))
    }

    /// Append a transparency group (ISO 32000-2 §11.4): everything `body` draws
    /// composites onto the backdrop [`GroupSpec::isolated`] names, and the finished
    /// group is painted exactly once under `spec`. Nesting is bounded at
    /// [`MAX_GROUP_DEPTH`](super::MAX_GROUP_DEPTH).
    ///
    /// `body` returns a `Result` so builder refusals inside the group propagate with
    /// `?` — a small departure from the brief's illustrative closure, in exchange for
    /// no refusal ever being swallowed.
    ///
    /// # Errors
    ///
    /// Refuses an invalid alpha, an unknown clip, nesting beyond the bound, a
    /// non-isolated group in a position §11.4.4's arithmetic cannot survive
    /// ([`SceneError::NonIsolatedGroupUnsupported`]), and whatever `body` itself
    /// refuses. On any error the group is discarded whole; the builder remains usable
    /// and consistent.
    pub fn group(
        &mut self,
        spec: GroupSpec,
        body: impl FnOnce(&mut Self) -> Result<(), SceneError>,
    ) -> Result<(), SceneError> {
        Self::check_alpha(spec.alpha)?;
        self.check_clip(spec.clip)?;
        self.check_mask(spec.mask)?;
        Self::check_group_compose(&spec)?;
        self.check_isolation(&spec)?;
        let commands = self.nested_body(self.inside_knockout() || spec.knockout, body)?;
        self.push(Command::Group { spec, commands });
        Ok(())
    }

    /// Append an image draw (ISO 32000-2 §8.9.5): the uploaded image mapped into
    /// the unit square under `transform`, under a constant alpha and the placement's
    /// **resolved** filter (integration note 1). The image identifier is opaque
    /// here; the device validates it at render time.
    ///
    /// # Errors
    ///
    /// Refuses non-finite transforms, an alpha outside `0..=1`, and clip or mask
    /// identifiers this scene never allocated.
    #[allow(clippy::too_many_arguments)] // the brief's §2.3 signature, mask last (note 8)
    pub fn image(
        &mut self,
        image: ImageId,
        transform: Affine,
        alpha: f32,
        filter: ImageFilter,
        clip: Option<ClipId>,
        blend: BlendMode,
        mask: Option<MaskId>,
    ) -> Result<(), SceneError> {
        Self::check_transform(transform)?;
        Self::check_alpha(alpha)?;
        self.check_clip(clip)?;
        self.check_mask(mask)?;
        self.push(Command::Image {
            image,
            transform,
            alpha,
            filter,
            clip,
            blend,
            mask,
        });
        Ok(())
    }

    /// Define a soft mask (ISO 32000-2 §11.5): `body` draws the mask's transparency
    /// group exactly as it would draw page content; the device renders it at device
    /// resolution and reduces it by `kind`, then `transfer` when given. Returns the
    /// identifier commands and groups reference it by — scene-scoped, and only valid
    /// *after* this call, which is what keeps mask dependencies acyclic.
    ///
    /// # Errors
    ///
    /// Refuses nesting beyond [`MAX_GROUP_DEPTH`](super::MAX_GROUP_DEPTH) and whatever
    /// `body` itself refuses; on any error no mask is defined.
    pub fn mask(
        &mut self,
        kind: MaskKind,
        transfer: Option<Transfer>,
        body: impl FnOnce(&mut Self) -> Result<(), SceneError>,
    ) -> Result<MaskId, SceneError> {
        Self::check_mask_kind(kind)?;
        // §11.6.5 renders the mask group on its own, so whatever encloses the `mask()`
        // call is not above the mask's content: the knockout stack starts fresh here.
        let commands = self.nested_body(false, body)?;
        let id = u32::try_from(self.masks.len()).unwrap_or(u32::MAX);
        self.masks.push(MaskDef {
            kind,
            transfer,
            commands,
        });
        Ok(MaskId(id))
    }

    /// Finish building. Consumes the builder; the scene is immutable from here on,
    /// and its [`Cost`](super::Cost) is computed here, once.
    #[must_use]
    pub fn finish(self) -> Scene {
        debug_assert!(
            self.open_frames.is_empty(),
            "group() and mask() close every frame they open, on both paths"
        );
        let cost = cost::measure(&self.commands, &self.clips, &self.masks);
        Scene {
            data: Arc::new(SceneData {
                commands: self.commands,
                clips: self.clips,
                masks: self.masks,
                cost,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SceneBuilder;
    use crate::blend::{BlendMode, Compose, FillRule};
    use crate::geom::{Affine, Point, Rect};
    use crate::ids::OutlineId;
    use crate::paint::{Color, Paint};
    use crate::scene::fixtures::black;

    /// An empty rectangle is accepted — it draws nothing, and drawing nothing is a
    /// legitimate thing for a scene to ask.
    #[test]
    fn empty_rect_is_accepted() {
        let mut builder = SceneBuilder::new();
        let empty = Rect::new(Point::new(3.0, 4.0), Point::new(3.0, 9.0));
        builder
            .rect(
                empty,
                Affine::IDENTITY,
                Color::new(1.0, 0.0, 0.0, 1.0),
                None,
                None,
            )
            .expect("an empty rect is legitimate");
        assert_eq!(builder.finish().commands().len(), 1);
    }

    /// A chain is an intersection: parents link backwards, and the definitions come
    /// back exactly as allocated.
    #[test]
    fn clip_chains_link_as_allocated() {
        let mut builder = SceneBuilder::new();
        let root = builder
            .clip(OutlineId(3), Affine::IDENTITY, FillRule::NonZero, None)
            .expect("valid clip");
        let child = builder
            .clip(
                OutlineId(4),
                Affine::scale(2.0, 2.0),
                FillRule::EvenOdd,
                Some(root),
            )
            .expect("valid child clip");
        builder
            .fill(
                OutlineId(5),
                Affine::IDENTITY,
                FillRule::NonZero,
                Paint::Solid(black()),
                Some(child),
                BlendMode::Multiply,
                Compose::SrcOver,
                None,
            )
            .expect("valid fill under a chain");
        let scene = builder.finish();
        assert_eq!(scene.clips().len(), 2);
        assert_eq!(
            scene.clips()[usize::try_from(child.0).unwrap()].parent,
            Some(root)
        );
        assert_eq!(scene.cost().clips, 2);
    }
}
