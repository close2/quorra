//! Turning an [`Archetype`] into the resources a device must hold and the scene that
//! draws it.
//!
//! The split into [`outlines`] / [`image_spec`] and [`scene`] is what keeps this crate
//! free of `quorra-gpu`: the caller uploads, this crate builds. The *order* of the
//! uploads is part of the contract, because a page's counters are a function of which
//! outline identifier each command names — see [`outlines`].

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use std::sync::Arc;

use quorra_scene::{
    Affine, BlendMode, ClipId, Color, Compose, FillRule, GroupSpec, ImageFilter, ImageId,
    ImageSpec, LineCap, LineJoin, OutlineId, Paint, Scene, SceneBuilder, SceneError, Segment,
    Stroke,
};

use crate::archetype::{Archetype, clip_of, curve_clip, outline_of, outline_side, position};

/// The ink every archetype draws in.
const INK: Color = Color::new(0.12, 0.13, 0.16, 1.0);

/// The outline paths an archetype needs uploaded, **in the order their identifiers are
/// expected in**.
///
/// The first `distinct.max(1)` entries are the page's marks, one per distinct outline.
/// A page whose clips are axis-aligned rectangles carries **one more** entry after them:
/// the unit rectangle every one of its clips is a scaling of. [`scene`] indexes both by
/// that convention, so a caller uploads this list front to back and passes the
/// identifiers in the same order.
#[must_use]
pub fn outlines(shape: &Archetype) -> Vec<Vec<Segment>> {
    let mut paths: Vec<Vec<Segment>> = (0..shape.distinct.max(1))
        .map(|i| outline_of(shape.segments, outline_side(shape, i)))
        .collect();
    if shape.rect_clips {
        paths.push(crate::archetype::rect_path(1.0));
    }
    paths
}

/// The synthetic image an archetype places, if it places any.
///
/// Its texels are a fixed ramp modulo 251, so the image is the same bytes on every run —
/// an image whose content varied would move `bytes_uploaded` without moving the page.
#[must_use]
pub fn image_spec(shape: &Archetype) -> Option<ImageSpec> {
    (shape.images > 0).then(|| {
        let side = shape.image_side.max(1);
        let pixels: Vec<u8> = (0..side * side * 4).map(|i| (i % 251) as u8).collect();
        ImageSpec {
            width: side,
            height: side,
            data: Arc::from(pixels.into_boxed_slice()),
        }
    })
}

/// Build the archetype's scene from resources a device has already accepted.
///
/// `outlines` must be the identifiers of [`outlines`]`(shape)`, in that order; `image`
/// the identifier of [`image_spec`]`(shape)`, or `None` for a page that places none.
///
/// Deterministic: the same archetype produces the same scene, command for command, on
/// every run and every machine.
///
/// # Errors
///
/// Whatever `SceneBuilder` refuses. Nothing here constructs a refusable scene, so an
/// error means the builder's own rules moved — which is worth propagating rather than
/// hiding, since every caller of this function is a gate or an instrument.
///
/// # Panics
///
/// If `outlines` is shorter than [`outlines`]`(shape)`. That is a contract violation at
/// the call site rather than anything a page can cause, and it names both lengths.
pub fn scene(
    shape: &Archetype,
    outlines: &[OutlineId],
    image: Option<ImageId>,
) -> Result<Scene, SceneError> {
    let distinct = shape.distinct.max(1) as usize;
    assert!(
        outlines.len() >= distinct + usize::from(shape.rect_clips),
        "{}: {} outline identifiers for a page that uploads {}",
        shape.name,
        outlines.len(),
        distinct + usize::from(shape.rect_clips),
    );
    // The marks' outlines and the clip rectangle are separate lists on purpose: a
    // command names `marks[i % marks.len()]`, and folding the rectangle into that
    // modulus would silently renumber every command on a rect-clipped page.
    let (marks, rectangle) = (&outlines[..distinct], outlines.get(distinct).copied());
    let mut builder = SceneBuilder::new();
    let clips = define_clips(&mut builder, shape, marks, rectangle)?;
    emit_groups(&mut builder, shape, marks, &clips)?;
    emit_images(&mut builder, shape, image)?;
    Ok(builder.finish())
}

/// The archetype's clip chains.
///
/// A clip is either an axis-aligned rectangle, which ADR 0007 resolves at encode time,
/// or a curve, which leaves a residue that every clipped command multiplies into a
/// coverage tile. Both are ordinary on real pages and they are different lanes with
/// different costs, which is why `rect_clips` is a field rather than a constant.
///
/// A rectangular clip here covers the page and differs from its neighbours by a hair, so
/// it admits every command under it: the subject is the resolver and
/// `clip_distinct_regions`, not culling, which has a gate of its own.
///
/// **A curve clip is cut around the run of marks that draw under it** ([`clip_of`]): the
/// ellipse is scaled onto their box, so it is three or four marks across, a fraction of
/// the page, and it cuts every mark under it — which is the `q W n` shape ADR 0049 and
/// ADR 0057 both exist for, and the one a clip on a grid of its own never was.
fn define_clips(
    builder: &mut SceneBuilder,
    shape: &Archetype,
    marks: &[OutlineId],
    rectangle: Option<OutlineId>,
) -> Result<Vec<ClipId>, SceneError> {
    let rectangle = rectangle.filter(|_| shape.rect_clips);
    let centre = Affine::translate(shape.width as f32 * 0.5, shape.height as f32 * 0.5);
    (0..shape.clips)
        .map(|i| {
            let outline = rectangle.unwrap_or(marks[(i as usize) % marks.len()]);
            let transform = if shape.rect_clips {
                let half = shape.height as f32 * 0.6 + i as f32 * 0.01;
                Affine::scale(half, half).then(centre)
            } else {
                curve_clip(shape, i)
            };
            builder.clip(outline, transform, FillRule::NonZero, None)
        })
        .collect()
}

/// The page's drawing commands: groups take the first of them, in equal runs, so nesting
/// is real rather than a wrapper around nothing — and `grouped` is derived from
/// `per_group`, so the totals are exact rather than rounded.
fn emit_groups(
    builder: &mut SceneBuilder,
    shape: &Archetype,
    outlines: &[OutlineId],
    clips: &[ClipId],
) -> Result<(), SceneError> {
    let per_group = (shape.commands / 4)
        .checked_div(shape.groups)
        .map_or(0, |per| per.max(1));
    let grouped = per_group * shape.groups;
    for group in 0..shape.groups {
        let spec = GroupSpec {
            alpha: 0.8,
            blend: if group < shape.blended_groups {
                BlendMode::Multiply
            } else {
                BlendMode::Normal
            },
            clip: None,
            knockout: false,
            mask: None,
            isolated: true,
            compose: Compose::SrcOver,
        };
        builder.group(spec, |body| {
            for step in 0..per_group {
                emit(body, shape, outlines, clips, group * per_group + step)?;
            }
            Ok(())
        })?;
    }
    for index in grouped..shape.commands {
        emit(builder, shape, outlines, clips, index)?;
    }
    Ok(())
}

/// The page's image placements, each the uploaded image mapped over one grid cell.
fn emit_images(
    builder: &mut SceneBuilder,
    shape: &Archetype,
    image: Option<ImageId>,
) -> Result<(), SceneError> {
    let Some(image) = image else { return Ok(()) };
    for index in 0..shape.images {
        let side = shape.image_side as f32;
        builder.image(
            image,
            Affine::scale(side, side).then(position(shape, index, side)),
            1.0,
            ImageFilter::Nearest,
            None,
            BlendMode::Normal,
            None,
        )?;
    }
    Ok(())
}

/// One drawing command: a stroke while the archetype's stroke budget lasts, a fill
/// after it, under the clip its index selects.
fn emit(
    builder: &mut SceneBuilder,
    shape: &Archetype,
    outlines: &[OutlineId],
    clips: &[ClipId],
    index: u32,
) -> Result<(), SceneError> {
    let outline = outlines[(index as usize) % outlines.len()];
    let clip = (index < shape.clipped && !clips.is_empty())
        .then(|| clips[clip_of(shape, index).min(clips.len() - 1)]);
    let at = position(shape, index, shape.side);
    if index < shape.strokes {
        builder.stroke(
            outline,
            at,
            Stroke {
                width: 1.5,
                adjust: false,
                cap: LineCap::Butt,
                join: LineJoin::Miter,
                miter_limit: 4.0,
            },
            Paint::Solid(INK),
            clip,
            BlendMode::Normal,
            None,
        )
    } else {
        builder.fill(
            outline,
            at,
            FillRule::NonZero,
            Paint::Solid(INK),
            clip,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
    }
}
