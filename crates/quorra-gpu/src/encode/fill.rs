//! The fill arm, and the choice of lane that is the whole of what it does.
//!
//! A fill is the command a document draws almost everything with, and nothing about it
//! says which lane should draw it: the same `Command::Fill` is a glyph at nine pixels,
//! a rectangle stated as four edges (ADR 0047), a page-sized clip shape, or a gradient
//! over an outline. So this file is one decision taken in one place, in the order the
//! conditions can be asked in — visibility before the implicit group a non-`Normal`
//! blend would wrap the fill in (§11.3.5), the rectangle lane before any coverage is
//! considered, then [`Encoder::fill_solid`]'s three: the device lane for a tile the
//! cache is no use for, the atlas for one it will hold and re-read, the sheet for
//! everything left.
//!
//! [`SolidFill`] exists for the same reason the decision does: the arm asks the cache
//! one question and hands the same fill to whichever of three lanes answers, and
//! threading the fill's own description through each of them by hand is how two of them
//! come to disagree about it.

use quorra_scene::{Affine, BlendMode, ClipId, Compose, FillRule, MaskId, OutlineId, Paint};

use super::clips::ResolvedClip;
use super::device_space::{apply, compose, tile_side, transform_preserves_axes};
use super::parallel::{Draw, Job};
use super::rare::RarePaint;
use super::thin::ThinAxis;
use super::{ChildOp, DrawStyle, Encoder, Op};
use crate::atlas::GlyphPlacement;
use crate::error::RenderError;
use crate::raster::{self, DeviceTransform, Polyline, Rule};
use crate::startup::Coverage;

/// One solid fill, resolved as far as the lane choice needs it.
///
/// A struct rather than nine parameters: the arm that draws it asks the cache one
/// question and then hands the same fill to whichever of three lanes answers, and
/// threading the fill's own description through each of them by hand is how two of them
/// come to disagree about it.
///
/// The lifetime is the **resource store's**, not the encoder's borrow: it carries the
/// outline the caller already looked up, which is what keeps the hottest walk in the
/// tree to one hash probe per solid fill instead of two ([`Encoder::fill_solid`]).
struct SolidFill<'a> {
    outline: OutlineId,
    /// The outline's resident geometry, looked up once by [`Encoder::encode_fill`].
    ///
    /// `&'a StoredOutline` and not a second `OutlineId` lookup: this is a borrow of the
    /// store, which the encoder holds as `&'a ResourceStore` for the whole frame, so it
    /// is independent of the `&mut self` the three lanes below need. Measured on the
    /// caller's 58 009-mark page (callgrind, `doc/notes-fill-solid-lookup.md`): the
    /// second probe was **9.5 M instructions, 2.3 % of that page's `recording`**, and
    /// **5.8 % of the dense-text archetype's**.
    stored: &'a crate::resources::StoredOutline,
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

/// One §10.7.4 mark's ink: the command's own paint, resolved as far as a mark needs.
#[derive(Clone, Copy)]
enum MarkInk {
    /// A solid colour, which may take the analytic rectangle lane.
    Solid(quorra_scene::Color),
    /// A resolved rare paint, which goes over a coverage quad.
    Rare(RarePaint),
}

/// The snapped mark: the run of whole device pixels the collapsed axis passes
/// through, as a device rectangle — the caller's `snapped_span` without the
/// round-trip through the placement's inverse, since this side's output is already
/// device geometry.
///
/// `None` where the device box is degenerate in neither axis or in both, which an
/// axis-preserving placement of a subpath collapsed along exactly one axis cannot
/// produce — the same guard as theirs, kept because reading it off the arithmetic is
/// cheaper than trusting the table's, and a mark on the wrong axis would be a line
/// across the page. The caller then falls back to the band, and so does this side.
#[expect(
    clippy::float_cmp,
    reason = "exactness identifies the collapsed axis, as in the caller's snapped_span: an \
              axis-preserving placement carries a shared coordinate to a shared coordinate \
              through the same arithmetic"
)]
fn snapped_mark(
    mark: &crate::resources::CollapsedMark,
    to_device: &DeviceTransform,
) -> Option<quorra_scene::Rect> {
    let a = apply(to_device, mark.min);
    let b = apply(to_device, mark.max);
    let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
    let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
    let (lo, hi) = if x0 == x1 && y0 != y1 {
        let column = x0.floor();
        (
            quorra_scene::Point::new(column, y0),
            quorra_scene::Point::new(column + 1.0, y1),
        )
    } else if y0 == y1 && x0 != x1 {
        let row = y0.floor();
        (
            quorra_scene::Point::new(x0, row),
            quorra_scene::Point::new(x1, row + 1.0),
        )
    } else {
        return None;
    };
    if !lo.x.is_finite() || !lo.y.is_finite() || !hi.x.is_finite() || !hi.y.is_finite() {
        return None;
    }
    Some(quorra_scene::Rect::new(lo, hi))
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

impl<'a> Encoder<'a> {
    /// The fill arm of the command walk: pick the glyph or path lane by device size
    /// and residue state; route non-Normal blends through an implicit child layer;
    /// mark `Compose::Src` for the knockout two-pass (§4.1).
    #[allow(clippy::too_many_arguments)] // one command's fields, destructured once
    pub(super) fn encode_fill(
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
        // The compute lane bounds by the resident control box's corners — for an
        // axis-preserving transform the same box to the bit, for a rotated one a
        // superset whose extra pixels read zero coverage (ADR 0083) — because the
        // direct hull walks every control point per placement, and on a page of
        // distinct outlines that walk *was* the encode.
        let bounds = if self.coverage == Coverage::Compute
            && matches!(paint, Paint::Solid(_))
            && resolved.residues.is_none()
        {
            Some(corner_bounds(stored.control_box, &to_device))
        } else {
            self.hulls.bounds(outline, &stored.segments, &to_device)
        };
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
            // ISO 32000-2 §10.7.4's marks for the subpaths that enclose no area,
            // placed per viewport from the resident collapse table — before the
            // fill itself, whose area rules paint nothing of them at any scale.
            let ink = MarkInk::Solid(color);
            self.encode_collapsed_marks(&stored.marks, &to_device, &resolved, ink, style, mask)?;
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
                    Some(device_rect) => {
                        self.push_rect_instance(device_rect, color, style, mask)?;
                    }
                    // The cull above tests bounds inflated by `CULL_MARGIN`; this one
                    // is exact, so it can still fire, and a command that reaches no
                    // pixel is counted wherever it is discovered.
                    None => self.note_culled(),
                }
                return Ok(());
            }
            let placement = SolidFill {
                outline,
                stored,
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
        // Shading, mesh or function paint (§8.7.4.5): one quad over a coverage source.
        // The rect-hinted case needs no scratch tile — analytic coverage, mirroring the
        // rectangle lane (ADR 0011).
        let Some(rare) = self.rare_paint(paint)? else {
            return Ok(());
        };
        // The paint is resolved, so the fill may now be dropped for being out of
        // sight — the half of the solid lane's test that had to wait.
        if bounds.is_none_or(|b| self.culled(b, &resolved)) {
            return Ok(());
        }
        // §10.7.4's marks again, with the command's own paint — the clause names the
        // shape, not the ink (the caller's `pdf_render::collapsed` fills its marks
        // with the command's paint for the same reason).
        let ink = MarkInk::Rare(rare);
        self.encode_collapsed_marks(&stored.marks, &to_device, &resolved, ink, style, mask)?;
        if resolved.residues.is_none()
            && transform_preserves_axes(&to_device)
            && let Some(rect) = stored.rect_hint
        {
            return self.push_rare_rect(rare, rect, &to_device, &resolved, style, mask);
        }
        let span = self.clock.start();
        let polylines = raster::flatten(&stored.segments, to_device);
        self.clock.geometry(span);
        self.push_rare_coverage(rare, &polylines, rule, &resolved, style, mask)
    }

    /// The marks ISO 32000-2 §10.7.4 asks for, one per collapsed subpath, placed on
    /// this viewport's pixel grid from the resident collapse table
    /// ([`StoredOutline::marks`](crate::resources::StoredOutline)).
    ///
    /// > A shape shall be scan-converted by painting any pixel whose half-open square
    /// > region intersects the shape, no matter how small the intersection is. This
    /// > ensures that no shape ever disappears as a result of unfavourable placement
    /// > relative to the device pixel grid […] A zero-width or zero-height rectangle
    /// > paints a line 1 pixel wide. (ISO 32000-2 §10.7.4)
    ///
    /// The arithmetic mirrors the caller's `pdf_render::collapsed` statement for
    /// statement, as [`raster::resolve_width`] mirrors their width resolution
    /// (ADR 0085) — their CPU oracle keeps the original and the cross-lane gates
    /// compare the two continuously:
    ///
    /// - Under an **axis-preserving, invertible** placement, the mark is the run of
    ///   whole device pixels the collapsed axis passes through: the axis's device
    ///   coordinate `v` lies in row or column `floor(v)`, and the mark spans
    ///   `floor(v)` to `floor(v) + 1` while the other axis keeps the subpath's own
    ///   extent (their `snapped_span` — stated here directly in device space, where
    ///   they round-trip through the placement's inverse because their output is path
    ///   geometry; the difference is ulps on coordinates that are exact integers
    ///   here, inside ADR 0082's contract).
    /// - Otherwise — a rotation, a shear, a placement with no inverse left to name a
    ///   grid — the mark is a **band** one device pixel thick about the subpath's own
    ///   line, stated in outline space as `1 / max_stretch` (their `thinnest_line`)
    ///   and carried through the placement (their `band_span`).
    ///
    /// No stretch at all — a placement that lengthens nothing — leaves no space for a
    /// mark to be stated in, and makes none: their `split_collapsed_fill` returns
    /// nothing on the same test.
    fn encode_collapsed_marks(
        &mut self,
        marks: &[crate::resources::CollapsedMark],
        to_device: &DeviceTransform,
        resolved: &ResolvedClip,
        ink: MarkInk,
        style: DrawStyle,
        mask: Option<u32>,
    ) -> Result<(), RenderError> {
        // Empty for almost every outline, and first so the hottest walk pays one
        // branch: the collapse table is resident precisely so this costs nothing
        // where there is nothing.
        if marks.is_empty() {
            return Ok(());
        }
        let stretch = to_device.max_stretch();
        if !stretch.is_finite() || stretch <= 0.0 {
            return Ok(());
        }
        // The grid exists where a device axis is an outline axis and the placement
        // has an inverse — the same two conditions as the caller's, though only the
        // first is arithmetic this side needs: a singular placement has flattened
        // the page onto a line, so there is no pixel grid for the snap to be right
        // about, and the band draws what the oracle draws.
        let determinant = to_device.a * to_device.d - to_device.b * to_device.c;
        let snappable =
            transform_preserves_axes(to_device) && determinant.is_finite() && determinant != 0.0;
        for mark in marks {
            let device_rect = if snappable {
                snapped_mark(mark, to_device)
            } else {
                None
            };
            if let Some(rect) = device_rect {
                // The snapped mark is axis-aligned in device space by construction,
                // so it takes the analytic lane where a solid rectangle would
                // (ADR 0007) and the coverage quad where it cannot.
                if let (MarkInk::Solid(color), None) = (ink, &resolved.residues) {
                    let clipped = rect.intersection(resolved.rect).intersection(self.visible);
                    if !clipped.is_empty() {
                        self.push_rect_instance(clipped, color, style, mask)?;
                    }
                    continue;
                }
                self.push_mark_corners(
                    [
                        rect.min,
                        quorra_scene::Point::new(rect.max.x, rect.min.y),
                        rect.max,
                        quorra_scene::Point::new(rect.min.x, rect.max.y),
                    ],
                    resolved,
                    ink,
                    style,
                    mask,
                )?;
                continue;
            }
            // The band: half the thinnest line each side of the collapsed axis, in
            // outline space, carried through the placement (their `band_span`).
            let half = (1.0 / stretch) / 2.0;
            let (lo, hi) = if mark.vertical {
                (
                    quorra_scene::Point::new(mark.min.x - half, mark.min.y),
                    quorra_scene::Point::new(mark.min.x + half, mark.max.y),
                )
            } else {
                (
                    quorra_scene::Point::new(mark.min.x, mark.min.y - half),
                    quorra_scene::Point::new(mark.max.x, mark.min.y + half),
                )
            };
            self.push_mark_corners(
                [
                    apply(to_device, lo),
                    apply(to_device, quorra_scene::Point::new(hi.x, lo.y)),
                    apply(to_device, hi),
                    apply(to_device, quorra_scene::Point::new(lo.x, hi.y)),
                ],
                resolved,
                ink,
                style,
                mask,
            )?;
        }
        Ok(())
    }

    /// One mark as four device corners through the coverage path — the route for a
    /// residue clip, a rare paint, or a band under an oblique placement. Non-finite
    /// corners make no mark, as in the caller's `append_mark`.
    fn push_mark_corners(
        &mut self,
        corners: [quorra_scene::Point; 4],
        resolved: &ResolvedClip,
        ink: MarkInk,
        style: DrawStyle,
        mask: Option<u32>,
    ) -> Result<(), RenderError> {
        if corners.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
            return Ok(());
        }
        let polylines = vec![Polyline {
            points: corners.to_vec(),
            closed: true,
        }];
        match ink {
            MarkInk::Solid(color) => self.push_coverage_styled(
                &polylines,
                Rule::NonZero,
                color,
                resolved,
                style,
                None,
                mask,
            ),
            MarkInk::Rare(rare) => {
                self.push_rare_coverage(rare, &polylines, Rule::NonZero, resolved, style, mask)
            }
        }
    }

    /// The solid arm of the fill walk: three lanes, and the cache decides between them.
    ///
    /// In order of preference, and each condition is stated where it is asked:
    /// [`Encoder::take_gpu_lane`] for a tile the cache is no use for, the glyph lane for
    /// one it will hold and re-read, the scratch path for everything left — a residue
    /// clip, or a tile too large for the atlas whose triangles cost more than its
    /// coverage.
    ///
    /// The outline arrives inside the [`SolidFill`] rather than being looked up again:
    /// [`Encoder::encode_fill`] has already taken the `UnknownOutline` refusal for this
    /// id, so a second lookup here could only ever succeed — and did, once per solid
    /// fill, on the hottest walk in the tree.
    fn fill_solid(
        &mut self,
        fill: &SolidFill<'a>,
        resolved: &ResolvedClip,
    ) -> Result<(), RenderError> {
        let stored = fill.stored;
        if self.coverage == Coverage::Compute && resolved.residues.is_none() {
            return self.fill_compute(fill, resolved);
        }
        let (bx0, by0, bx1, by1) = fill.bounds;
        let (tile_width, tile_height) = (tile_side(bx0, bx1), tile_side(by0, by1));
        // What the cache would do with this placement, asked once and answered by the
        // atlas and the census together (ADR 0029). Both lanes below read this one
        // answer: a lane chosen on one reading of the cache and taken on another is how
        // a tile ends up rasterised twice, or not at all.
        let placed_once =
            self.census
                .placed_once(fill.outline.0, linear_bits(fill.transform), fill.rule);
        let cache = self.prospect_for(
            GlyphPlacement::of(fill.outline, &fill.to_device, fill.rule, self.quantum),
            tile_width,
            tile_height,
            placed_once,
        )?;
        // The GPU lane takes the outline as quadratics, not polylines — which is the
        // whole of why its cost does not grow with the magnification: there is no
        // flattening here to be done again at a new scale, and no atlas in front of it
        // to be cold (ADR 0016).
        //
        // **The order of the three tests below is the optimisation** (ADR 0075). The
        // conversion into quadratics is the most expensive thing an outline ever costs —
        // 490 instructions per segment, 156 ms of the caller's 187.6 ms launch scene
        // phase when it ran at upload — and every test that can answer without it is
        // asked first. A host on `Coverage::Cpu` stops at `gpu_lane_admissible` and
        // converts nothing, ever; `examples/outline_upload.rs` is the measurement.
        if self.gpu_lane_admissible(
            resolved,
            cache,
            // A fill has no width of its own, so its device box is the whole measure —
            // and for a curve that box is the control hull's, which over-states a thin
            // curve's extent. Both limits are stated on [`ThinAxis`] (ADR 0070).
            ThinAxis::of(fill.bounds, None),
        ) {
            // Copied out of `self` so the borrow is the store's `'a` and not a reborrow
            // of the `&mut self` `push_gpu_tile` needs below.
            let resources = self.resources;
            let quads = stored.quads(resources.budget())?;
            // Emptiness and the triangle count were two questions of one converted form;
            // asking them in this order rather than before the conversion changes which
            // test answers first and not what the conjunction answers.
            if !quads.is_empty()
                && Self::triangles_under_coverage(tile_width, tile_height, quads.triangle_count())
            {
                let Some(tile) = self.visible_tile(fill.bounds, resolved) else {
                    return Ok(());
                };
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
        }
        // Cacheable is a question for the atlas — how much of it this tile would take —
        // rather than a constant here (ADR 0024). A residue chain still takes the
        // scratch path: the clip multiplies into the tile, so the tile is not the glyph
        // and would poison the cache for every other placement of it.
        if let (None, Some((placement, entry))) = (resolved.residues.as_ref(), cache.admission()) {
            // The tile is rasterised at the *quantised* phase and drawn at the integer
            // origin: that split is the whole of what the quantum does (ADR 0009).
            let tile_transform = DeviceTransform {
                e: placement.phase[0],
                f: placement.phase[1],
                ..fill.to_device
            };
            return self.enqueue(Job::glyph(
                &stored.segments,
                tile_transform,
                fill.rule,
                placement.key,
                placement.origin,
                entry,
                // The tile the atlas was just asked about, which is the hull's box and so
                // an upper bound on the one the rasteriser will make.
                u64::from(tile_width).saturating_mul(u64::from(tile_height)),
                Draw::new(fill.color, resolved.rect, fill.style, fill.mask),
            ));
        }
        if let Some(rect) = self.deferrable_bounds(resolved) {
            let bound = self.tile_bound(fill.bounds, resolved);
            return self.enqueue(Job::sheet(
                &stored.segments,
                fill.to_device,
                None,
                fill.rule,
                rect,
                bound,
                Draw::new(fill.color, resolved.rect, fill.style, fill.mask),
            ));
        }
        let span = self.clock.start();
        let polylines = raster::flatten(&stored.segments, fill.to_device);
        self.clock.geometry(span);
        self.push_coverage_styled(
            &polylines, fill.rule, fill.color, resolved, fill.style, None, fill.mask,
        )
    }

    /// The compute lane's route for one solid fill (ADR 0080, ADR 0081): no job and
    /// no flattening — the device holds the outline's segments resident and flattens
    /// them itself. The tile is the control hull's, a superset of the flattened
    /// geometry's, whose extra pixels read zero coverage. Taken before the cache is
    /// consulted, because there is no atlas in front of this lane.
    fn fill_compute(
        &mut self,
        fill: &SolidFill<'a>,
        resolved: &ResolvedClip,
    ) -> Result<(), RenderError> {
        let Some((left, top, width, height)) = self.visible_tile(fill.bounds, resolved) else {
            return Ok(());
        };
        self.charge_tile(width, height)?;
        // The lane's per-tile device bytes: the accumulator floats. The edge buffer is
        // counted by the device and checked there (ADR 0081), and the row jobs stopped
        // existing when the deposit pass learned to search (ADR 0083).
        let acc = u64::from(width.saturating_add(1))
            .saturating_mul(u64::from(height))
            .saturating_mul(4);
        self.charge(acc)?;
        let seat = self
            .scratch
            .reserve(width, height)
            .ok_or_else(|| self.scratch.exhausted(width, height))?;
        let device = fill.to_device;
        #[allow(clippy::cast_precision_loss)] // the corner is a device pixel, the
        // same cast the CPU rasteriser makes on the same value
        let corner = (left as f32, top as f32);
        if !self.compute.push_tile(
            seat,
            (corner.0, corner.1, width, height),
            fill.rule == Rule::EvenOdd,
            [device.a, device.b, device.c, device.d, device.e, device.f],
            fill.outline.0,
        ) {
            // Only reachable when a host stated a budget past what the lane's u32
            // counters can address; the refusal is the budget's shape because that
            // is what ran out.
            return Err(RenderError::FrameBudgetExceeded {
                needed: u64::from(u32::MAX),
                budget: self.budget,
            });
        }
        #[allow(clippy::cast_precision_loss)] // seats and extents are texels
        return self.push_quad_instance(
            quorra_scene::Point::new(corner.0, corner.1),
            width as f32,
            height as f32,
            seat.0 as f32,
            seat.1 as f32,
            super::instance::CoverageSource::Sheet,
            fill.color,
            resolved.rect,
            fill.style,
            fill.mask,
        );
    }
}

/// The box of a box under an affine: the four corners transformed and min/maxed —
/// exact for an axis-preserving transform by `hull.rs`'s monotonicity argument, a
/// superset under rotation (ADR 0083).
fn corner_bounds(control_box: [f32; 4], to_device: &DeviceTransform) -> (f32, f32, f32, f32) {
    let corners = [
        apply(
            to_device,
            quorra_scene::Point::new(control_box[0], control_box[1]),
        ),
        apply(
            to_device,
            quorra_scene::Point::new(control_box[2], control_box[1]),
        ),
        apply(
            to_device,
            quorra_scene::Point::new(control_box[0], control_box[3]),
        ),
        apply(
            to_device,
            quorra_scene::Point::new(control_box[2], control_box[3]),
        ),
    ];
    let mut bounds = (corners[0].x, corners[0].y, corners[0].x, corners[0].y);
    for corner in &corners[1..] {
        bounds = (
            bounds.0.min(corner.x),
            bounds.1.min(corner.y),
            bounds.2.max(corner.x),
            bounds.3.max(corner.y),
        );
    }
    bounds
}

impl Encoder<'_> {
    /// §11.3.5 for a single element: the implicit one-element group a blended fill
    /// draws through, so the blend function sees the element's own colour rather than
    /// the accumulated layer's.
    ///
    /// Inside a knockout group the element composites with the transparent initial
    /// backdrop, where every blend mode degenerates to Normal — §11.4.6 with §11.3.6's
    /// αb = 0 — so knockout draws never come here.
    #[allow(clippy::too_many_arguments)] // the fill's own parameters, forwarded once
    pub(super) fn fill_through_blend_group(
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
        self.push_op(Op::Child(ChildOp::implicit_blend_group(child, blend, mask)))
    }
}
