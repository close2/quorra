//! Where a mark's coverage comes from, and the one place that decides.
//!
//! Everything that is neither the analytic rectangle lane nor the atlas ends up here
//! with a shape in device space and needs an R8 tile for it. There are two ways to get
//! one and they are not alternatives a caller picks: the CPU rasteriser computes the
//! tile ([`Encoder::coverage_tile`]), or the device draws it from the outline's
//! triangles into the same sheet ([`Encoder::push_gpu_tile`]) — and
//! [`Encoder::take_gpu_lane`] is the four conditions that choose between them, each one
//! a measurement rather than a taste.
//!
//! The choice and both of its branches are one module deliberately. Splitting the
//! chooser from what it chooses is how a lane comes to be taken on one reading of the
//! cache and entered on another, which is a tile rasterised twice or not at all; and the
//! two branches have to agree, to the pixel, about the tile they produce — the same
//! `shape ∩ clip ∩ target` rounded out the same way — which is a property a reader can
//! only check by having both in front of them ([`Encoder::visible_tile`] is that
//! arithmetic without the rasterising).
//!
//! Whichever branch ran, the tile is packed onto the frame's sheet in encounter order
//! (ADR 0034) and drawn as one quad instance.

use quorra_scene::{Point, Rect};

use super::clips::ResolvedClip;
use super::device_space::tile_side;
use super::{DrawStyle, Encoder};
use crate::atlas::CacheProspect;
use crate::error::RenderError;
use crate::raster::{self, Polyline, Rule};
use crate::startup::Coverage;

impl Encoder<'_> {
    /// The path lane: rasterise coverage for these polylines over the visible
    /// region, multiply residue clips in, pack into scratch, emit the quad.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    #[allow(clippy::arithmetic_side_effects, clippy::cast_precision_loss)]
    pub(super) fn push_coverage(
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
    pub(super) fn push_coverage_styled(
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
    pub(super) fn coverage_tile(
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
    pub(super) fn visible_tile(
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
    pub(super) fn take_gpu_lane(
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
    pub(super) fn push_gpu_tile(
        &mut self,
        tile: (i32, i32, u32, u32),
        rule: Rule,
        color: quorra_scene::Color,
        resolved: &ResolvedClip,
        style: DrawStyle,
        mask: Option<u32>,
        triangles: impl FnOnce(&mut Vec<crate::outline::WindingVertex>, [f32; 2], [f32; 4]),
    ) -> Result<(), RenderError> {
        // A reservation is a shelf, and shelves are taken in encounter order (ADR 0034),
        // so anything queued takes its place first (`parallel`).
        self.drain_queue()?;
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
        )
    }

    /// Pack into scratch, charging is the caller's; splits from `push_scratch_quad`
    /// so residue planning can pack without emitting a quad.
    pub(super) fn pack_scratch(
        &mut self,
        tile: &raster::CoverageMask,
    ) -> Result<(u32, u32), RenderError> {
        // The sheet is packed in encounter order and ADR 0034 made that order
        // load-bearing, so anything queued takes its shelf first (`parallel`).
        self.drain_queue()?;
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
    pub(super) fn push_scratch_quad(
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
        )
    }
}
