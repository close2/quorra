//! A child layer: what becomes one, and the composite that puts it back.
//!
//! ISO 32000-2 §11.4.5 draws a transparency group into a buffer of its own and
//! composites the result once, under the group's blend mode, alpha, clip and soft mask.
//! Three things in a scene reach that: a `Command::Group`; an element whose blend mode
//! is not `Normal`, which §11.3.5 requires to see its *own* backdrop and so is wrapped
//! in an implicit one-element group; and a soft mask, whose group is realised as a plan
//! like any other and reduced before the frame draws (§11.5).
//!
//! They are one module because they are one clause read three ways, and because the
//! fields of a [`ChildOp`] are where the three would drift apart. The composite's own
//! refusal to run — a child whose marks cannot reach a pixel of the plan below it
//! (ADR 0041) — is here too, with the argument for why dropping it draws the same frame
//! written beside the drop rather than in an ADR nobody re-reads.
//!
//! What the compositor *does* with a `ChildOp` is `compose`'s half; what a group means
//! to the walk is this one.

use quorra_scene::{BlendMode, MaskId, MaskKind};

use super::clips::{OPEN_CLIP, ResolvedClip};
use super::{DrawStyle, Encoder, LayerPlan, Op};
use crate::error::RenderError;
use crate::raster;

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
    pub(super) fn implicit_blend_group(layer: usize, blend: BlendMode, mask: Option<u32>) -> Self {
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

/// §11.3.5's mode numbering for the composite shader: `BlendMode`'s declaration
/// order, which follows the clause's own table.
pub(super) fn blend_word(mode: BlendMode) -> u32 {
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

impl Encoder<'_> {
    /// Plan a child layer: run `body` with the current plan switched to a fresh
    /// node, restoring on both paths.
    ///
    /// The two drains are the plan boundary: a queued mark belongs to the plan that was
    /// current when the walk reached it, and this is the one place `current_plan` moves
    /// (`parallel`). The second drain runs *inside* the child, before the restore, and
    /// only when the body succeeded — a body that failed is a frame that will be refused.
    pub(super) fn plan_child(
        &mut self,
        body: impl FnOnce(&mut Self) -> Result<(), RenderError>,
    ) -> Result<usize, RenderError> {
        self.drain_queue()?;
        let child = self.layers.len();
        self.layers.push(LayerPlan::default());
        let outer = self.current_plan;
        self.current_plan = child;
        let result = body(self).and_then(|()| self.drain_queue());
        self.current_plan = outer;
        result?;
        Ok(child)
    }

    /// Realise a referenced soft mask's plan on first use; masks reference only
    /// earlier masks (the builder enforced it), so this terminates.
    pub(super) fn use_mask(&mut self, mask: Option<MaskId>) -> Result<Option<u32>, RenderError> {
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
    pub(super) fn plan_group_residue(
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
    pub(super) fn push_child(&mut self, child: ChildOp) {
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
}
