//! The encoder's side of the seam: what it queues, when it must stop queueing, and how
//! a rasterised job is placed.
//!
//! Split from its parent because they are two subjects. [`super`] is the *work* — a
//! `Job`, a pure function over one, and the arithmetic that divides a list of them — and
//! nothing in it can see a frame. This is the *frame*: the budget, the atlas, the sheet
//! and the instance stream, all of which are ordered and all of which stay on the
//! calling thread. A child module rather than a sibling, so `Job`'s fields stay private
//! to the pair (ADR 0051).
//!
//! Read [`super`]'s header first: the three phases, the drain rule and the atlas guard
//! are stated there, and this file is where two of the three happen.

use quorra_scene::{Point, Rect};

use crate::atlas::{AtlasEntry, CacheProspect, GlyphKey, GlyphPlacement};
use crate::error::RenderError;
use crate::startup::Coverage;

use super::super::instance::CoverageSource;
use super::super::{Encoder, ResolvedClip};
use super::{Draw, Job, Place, Rasterised, fan_out, rasterise, rasterise_all};

impl<'a> Encoder<'a> {
    /// What the atlas would do for this placement, asked of an atlas the queue has
    /// already reached.
    ///
    /// **The guard, and it belongs here rather than at [`Encoder::enqueue`].** A queued
    /// job has not inserted its key yet, so a repeat of that key would read `entry: None`
    /// and be *built* as a second rasterisation of one picture — and by the time the
    /// queue is drained the lane has already been chosen on the stale answer, which is
    /// exactly the "chosen on one reading, drawn on another" hazard
    /// [`AtlasStore::prospect`](crate::atlas::AtlasStore::prospect) is written against.
    /// Committing first is what the serial walk does: the repeat then finds the entry.
    ///
    /// Costs one lookup in an empty set per solid fill when no queue exists, and nothing
    /// at all when the host asked for one thread.
    pub(in crate::encode) fn prospect_for(
        &mut self,
        placement: GlyphPlacement,
        width: u32,
        height: u32,
        placed_once: bool,
    ) -> Result<CacheProspect, RenderError> {
        if self.threads > 1 && self.queued_keys.contains(&placement.key) {
            self.drain_queue()?;
        }
        Ok(self.atlas.prospect(placement, width, height, placed_once))
    }

    /// The largest coverage tile a mark with these bounds can make: the same arithmetic
    /// [`Encoder::visible_tile`] does, in bytes.
    ///
    /// `bounds` is the shape's control hull, so this is an **upper** bound on the tile
    /// the rasteriser will produce from the flattened geometry — which is what
    /// `Job::held` needs and the only thing about a mark's size that is known before it
    /// is flattened.
    pub(in crate::encode) fn tile_bound(
        &self,
        bounds: (f32, f32, f32, f32),
        resolved: &ResolvedClip,
    ) -> u64 {
        self.visible_tile(bounds, resolved)
            .map_or(0, |(_, _, width, height)| {
                u64::from(width).saturating_mul(u64::from(height))
            })
    }

    /// The rectangle a path-lane job rasterises over — clip ∩ target — or `None` when
    /// this mark may not leave the walk's thread.
    ///
    /// Two conditions, and each is a lane that has not been taught the seam rather than
    /// a limit of it:
    ///
    /// - **a residue clip** multiplies into the tile out of a cache the walk builds and
    ///   decides about as it goes (`super::super::residue`), which is shared mutable
    ///   state a fan-out would have to freeze first;
    /// - **[`Coverage::Gpu`]** asks [`Encoder::take_gpu_lane`] a second question about
    ///   the *flattened* triangle count, so a job that skipped the flattening would be
    ///   choosing its lane on one reading and drawing on another — the hazard ADR 0029
    ///   names.
    ///
    /// Folding the clip and the target together here is exact: `f32::max` is associative
    /// and returns its non-NaN operand, so `max(max(x0, clip), 0)` is
    /// `max(x0, max(clip, 0))` for every input, which is what makes this the same bound
    /// [`Encoder::coverage_tile`] computes in place.
    pub(in crate::encode) fn deferrable_bounds(&self, resolved: &ResolvedClip) -> Option<[f32; 4]> {
        (resolved.residues.is_none() && self.coverage == Coverage::Cpu)
            .then(|| self.visible_rect(resolved))
    }

    /// Clip ∩ target as a job's rectangle — the folded bound `deferrable_bounds`
    /// documents, shared with the compute lane's route (ADR 0080).
    #[allow(clippy::cast_precision_loss)] // a viewport extent, far inside f32's exact
    // integer range
    pub(in crate::encode) fn visible_rect(&self, resolved: &ResolvedClip) -> [f32; 4] {
        [
            resolved.rect.min.x.max(0.0),
            resolved.rect.min.y.max(0.0),
            resolved.rect.max.x.min(self.viewport.width as f32),
            resolved.rect.max.y.min(self.viewport.height as f32),
        ]
    }

    /// Take a job: queue it when the host asked for threads, run it here otherwise.
    pub(in crate::encode) fn enqueue(&mut self, job: Job<'a>) -> Result<(), RenderError> {
        if self.threads <= 1 {
            // The clock is opened here rather than around the whole of `enqueue`,
            // because what ADR 0023 calls geometry is the flattening and the scanline
            // pass and not the commit that follows them — `drain_queue` opens the same
            // span around the fan-out for the same reason.
            let span = if job.rasterises() {
                self.clock.start()
            } else {
                None
            };
            let mask = rasterise(&job);
            self.clock.geometry(span);
            return self.commit(&job, mask);
        }
        // **The queue is bounded in bytes, not only in jobs.** Where the walk held one
        // coverage tile at a time, a queue holds every tile in flight — so a page of
        // large marks could hold a gigabyte between the walk and the commit, un-charged,
        // which is principle 3's "never allocate from an unchecked number" arriving
        // through the back door. Draining first keeps the peak at one limit plus one job,
        // and a drain is always legal: it is the order the walk would have run in anyway.
        if !self.queue.is_empty()
            && self.queued_bytes.saturating_add(job.held()) > self.in_flight_limit
        {
            self.drain_queue()?;
        }
        if let Some(key) = job.key_written() {
            self.queued_keys.insert(key);
        }
        self.queued_weight = self.queued_weight.saturating_add(job.weight());
        self.queued_bytes = self.queued_bytes.saturating_add(job.held());
        self.queue.push(job);
        Ok(())
    }

    /// Rasterise everything queued and commit it, in encounter order.
    ///
    /// The queue is emptied **before** the commit runs, so the commit reaches the same
    /// ordinary methods the walk does and finds nothing to drain.
    pub(in crate::encode) fn drain_queue(&mut self) -> Result<(), RenderError> {
        if self.queue.is_empty() {
            return Ok(());
        }
        let jobs = std::mem::take(&mut self.queue);
        self.queued_keys.clear();
        self.queued_bytes = 0;
        let weight = std::mem::take(&mut self.queued_weight);
        let threads = fan_out(weight, self.threads);
        // The fan-out's own wall clock is this frame's geometry, whoever ran it: the
        // instrument's subject is what the frame spent, not what one thread did
        // (ADR 0023).
        let span = if weight > 0 { self.clock.start() } else { None };
        let masks = rasterise_all(&jobs, threads);
        self.clock.geometry(span);
        for (job, mask) in jobs.iter().zip(masks) {
            self.commit(job, mask)?;
        }
        Ok(())
    }

    /// Place one rasterised job: the third phase, in encounter order.
    fn commit(&mut self, job: &Job<'a>, mask: Rasterised) -> Result<(), RenderError> {
        match job.place {
            Place::Resident { key, origin, entry } => {
                self.commit_glyph(key, origin, Some(entry), None, &job.draw)
            }
            Place::Atlas { key, origin } => self.commit_glyph(key, origin, None, mask, &job.draw),
            Place::Sheet => self.commit_sheet(mask, &job.draw),
        }
    }

    /// The glyph lane's commit: the atlas is offered the tile, and the placement draws
    /// from wherever it ends up.
    ///
    /// `resident` and `tile` are the two ways a placement can have a picture and they
    /// are exclusive by construction — [`Job::glyph`] chooses one at the walk, which is
    /// the reading `AtlasStore::prospect` insists on being made once.
    #[allow(clippy::cast_precision_loss)] // a tile corner and an atlas extent are both
    // integers bounded by the device dimension, far inside f32's exact range
    fn commit_glyph(
        &mut self,
        key: GlyphKey,
        origin: [f32; 2],
        resident: Option<AtlasEntry>,
        tile: Option<crate::raster::CoverageMask>,
        draw: &Draw,
    ) -> Result<(), RenderError> {
        let [ix, iy] = origin;
        let first_use = self.atlas_keys.insert(key);
        let entry = if let Some(entry) = resident {
            if first_use {
                self.atlas_requested_bytes = self
                    .atlas_requested_bytes
                    .saturating_add(u64::from(entry.width).saturating_mul(u64::from(entry.height)));
            }
            Some(entry)
        } else {
            // No geometry: the placement marks nothing, and the key stays counted —
            // which is what the serial walk did, since it took the key before it knew.
            let Some(tile) = tile else { return Ok(()) };
            self.charge_tile(tile.width, tile.height)?;
            if first_use {
                self.atlas_requested_bytes = self
                    .atlas_requested_bytes
                    .saturating_add(u64::from(tile.width).saturating_mul(u64::from(tile.height)));
            }
            let span = self.clock.start();
            let inserted = self.atlas.insert(key, &tile);
            self.clock.staging(span);
            if inserted.is_none() {
                // Atlas full: this tile draws uncached, and the device repacks the atlas
                // after the frame. Same pixels either way — one rasteriser feeds both
                // paths.
                self.atlas_pressure = true;
                // Counted per *placement* and not per key, because what this measures is
                // the work the frame did rather than the keys it wanted: a mark whose key
                // another placement already failed to insert is rasterised, packed and
                // sampled all over again (`Counters::atlas_overflow_tiles`).
                self.atlas_overflow_tiles = self.atlas_overflow_tiles.saturating_add(1);
                let dest = Point::new(ix + tile.left as f32, iy + tile.top as f32);
                return self
                    .push_scratch_quad(&tile, dest, draw.color, draw.clip, draw.style, draw.mask);
            }
            inserted
        };
        // One count per distinct key that reached an entry, however it reached it
        // (ADR 0050).
        if first_use && entry.is_some() {
            self.atlas_entries_used = self.atlas_entries_used.saturating_add(1);
        }
        let Some(entry) = entry else { return Ok(()) };
        let dest = Point::new(ix + entry.tile_left as f32, iy + entry.tile_top as f32);
        let device_rect = Rect::new(
            dest,
            Point::new(dest.x + entry.width as f32, dest.y + entry.height as f32),
        );
        if device_rect.intersection(draw.clip).is_empty() {
            return Ok(());
        }
        self.push_quad_instance(
            dest,
            entry.width as f32,
            entry.height as f32,
            entry.x as f32,
            entry.y as f32,
            CoverageSource::Atlas,
            draw.color,
            draw.clip,
            draw.style,
            draw.mask,
        )
    }

    /// The path lane's commit: the tile is charged, packed and drawn.
    #[allow(clippy::cast_precision_loss)] // a tile's corner is an integer device pixel
    fn commit_sheet(
        &mut self,
        tile: Option<crate::raster::CoverageMask>,
        draw: &Draw,
    ) -> Result<(), RenderError> {
        let Some(tile) = tile else { return Ok(()) };
        self.charge_tile(tile.width, tile.height)?;
        let dest = Point::new(tile.left as f32, tile.top as f32);
        self.push_scratch_quad(&tile, dest, draw.color, draw.clip, draw.style, draw.mask)
    }
}
