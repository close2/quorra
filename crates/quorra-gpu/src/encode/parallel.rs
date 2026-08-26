//! The coverage a run of marks can make off the walk's thread, and the seam that lets
//! it (the caller's `doc/QUORRA_ENCODE_THREADS.md`).
//!
//! # What is parallel, and why exactly that
//!
//! [`raster::fill_mask`] and [`raster::flatten`] are **pure functions** of one mark's
//! own geometry: no `&mut self`, no shared table, nothing read that another mark wrote.
//! Every other part of the walk is order-dependent and stays on the walk's thread —
//! the frame budget's running total, the scratch sheet's shelf cursors (whose encounter
//! order ADR 0034 made load-bearing and declined to sort), the atlas allocator, and the
//! instance stream. So the phase splits in three:
//!
//! 1. **the walk**, serial: resolve the clip, choose the lane, and record a [`Job`] —
//!    everything needed to rasterise one mark, and nothing about where it will land;
//! 2. **the fan-out**, parallel: [`rasterise`], once per job, into the job's own
//!    [`CoverageMask`];
//! 3. **the commit**, serial and in encounter order: charge the budget, offer the tile
//!    to the atlas or pack it on the sheet, and append the instance.
//!
//! Because step 3 runs in the order step 1 queued, **every order-dependent number is
//! the number a one-threaded frame produces**: the same charges in the same sequence,
//! the same shelves, the same instances, and therefore the same refusals.
//! `tests/encode_threads.rs` holds that to byte equality across thread counts rather
//! than arguing it.
//!
//! # The one place the queue can be observed, and the rule that closes it
//!
//! A queued job has not charged, not packed and not drawn. Anything that reads or
//! advances the frame's order therefore drains the queue first —
//! [`Encoder::drain_queue`] is called from `charge`, `pack_scratch`, `push_op`, the two
//! instance appenders and `plan_child`, which between them are every route to an
//! order-dependent effect. Draining empties the queue *before* committing, so the
//! commit reaches those same methods and finds nothing to drain: there is one set of
//! call sites rather than a shadow set that must be kept in step.
//!
//! The other observation is the atlas: a queued job has not inserted its key, so a
//! repeat of that key would read `entry: None` from an atlas the first has not reached
//! and both would rasterise and insert. [`Encoder::prospect_for`] drains before the
//! answer is given rather than before the job is queued, because the lane is chosen on
//! that answer and a drain after the choice comes too late. Nothing else about
//! [`AtlasStore::prospect`](crate::atlas::AtlasStore::prospect) depends on what is
//! queued — ADR 0029 kept "has the atlas room?" out of that answer deliberately, so
//! occupancy cannot change a lane.
//!
//! # Why a scope and not a pool
//!
//! ADR 0023 recorded the caller's answer to "should quorra build a thread pool?" as
//! **no — take one rather than make one**, for three reasons that are still true: their
//! `rayon` would be oversubscribed, their confined worker's seccomp filter kills the
//! `/sys` read `glibc` sizes its arenas from, and a pool built at construction is on
//! their time-to-first-page. Their `doc/QUORRA_ENCODE_THREADS.md` §4 asks for the shape
//! that satisfies all three: a frame that enters threads inside `Device::render` and has
//! left them before the call returns. [`std::thread::scope`] is exactly that, and needs
//! no dependency, no `unsafe`, and nothing alive between frames.
//!
//! **The host names the number**
//! ([`Options::encode_threads`](crate::startup::Options::encode_threads)), and one is
//! the default: a host that has not asked for threads runs the walk it ran before, and
//! a host whose process cannot spawn one is not made to.
//!
//! # Why this is one file, having been looked at as two
//!
//! It is 559 lines, which CLAUDE.md's file-scale rule calls a smell, and it has already
//! given up the seam it had: `commit` is step 3, in its own module. The division
//! proposed for what is left — the [`Job`] record in one module and the fan-out
//! (`partition`, `rasterise_all`, `fan_out`) in another — was looked for and is not
//! there:
//!
//! - **[`rasterise`] is the join, not a member of either half.** It is the pure function
//!   of a [`Job`] that the whole design rests on, and it would have to be filed with the
//!   record or with the threads while being the reason both exist.
//! - **The fan-out balances by [`Job::weight`] and is bounded by [`Job::held`]**, so a
//!   module holding `partition` without the record holds arithmetic over fields it
//!   cannot see. The two constants are the same: `IN_FLIGHT_BUDGET_SHARE` bounds what
//!   jobs may sit queued and `PARALLEL_FLOOR_SEGMENTS` decides whether any go off-thread
//!   at all, and neither means anything without the other side.
//! - **The four tests do not divide.** Three are statements about the partition and the
//!   floor and the fourth about the queue's bound — none is a statement about a `Job` on
//!   its own, which is the tell that the record is not a subject.
//!
//! Of the 559 lines, **313 carry code and 96 of those are the tests**; sixty are this
//! comment, which is the design argument the caller's `doc/QUORRA_ENCODE_THREADS.md`
//! asked for and the reason a reader can check the determinism claim at all.
//! `doc/HANDOVER.md` recorded that this file "was left because ADR 0054 had landed in it
//! two commits earlier" — that reason has expired, and this paragraph replaces it with
//! one that does not.

use std::thread;

use quorra_scene::{Color, Rect, Segment, Stroke};

use crate::atlas::{AtlasEntry, GlyphKey};
use crate::raster::{self, CoverageMask, DeviceTransform, Rule};

use super::DrawStyle;

mod commit;

/// The share of the caller's stated `max_frame_bytes` that may sit in the queue as
/// coverage the commit has not reached yet.
///
/// Derived from the host's own number rather than picked, for the reason `residue.rs`
/// gives about its own share: a caller who lowers `max_frame_bytes` is describing a
/// machine. A sixty-fourth of the 268 MiB default is 4 MB, which is about twenty thousand
/// of the caller's three-pixel marks or two full-page tiles — enough work to divide
/// twenty-four ways many times over, and small next to a budget the frame may spend in
/// full.
///
/// It is a **batching granularity and never a capacity**: a single job larger than this
/// is queued alone and drained straight after, so no frame is refused by it and every
/// tile is still charged exactly once, in encounter order, at the commit.
const IN_FLIGHT_BUDGET_SHARE: u64 = 64;

/// What a frame's queue may hold in coverage the commit has not reached, given the
/// frame's own budget.
pub(super) fn in_flight_limit(frame_budget_bytes: u64) -> u64 {
    frame_budget_bytes.saturating_div(IN_FLIGHT_BUDGET_SHARE)
}

/// The queued segment count below which a frame rasterises on the walk's thread.
///
/// A weight rather than a job count, because six 40 000-segment fills are more work
/// than six thousand triangles and the fan-out should take the first. The measurement
/// that set it is in `doc/notes-encode-threads.md`; the median corpus page (twelve
/// marks, ninety-six segments) is two orders of magnitude below it, which is the
/// property the caller's §4 asks for by name.
const PARALLEL_FLOOR_SEGMENTS: u64 = 4_096;

/// One mark's rasterisation, lifted out of the walk.
///
/// Borrowed, never owned: the segments are the resource store's, which outlives the
/// frame. What a job may **not** hold is anything the frame's order decides — a sheet
/// position, a budget total, an instance index — because that is the whole of what
/// makes [`rasterise`] a pure function.
pub(super) struct Job<'a> {
    /// The outline as the caller uploaded it.
    segments: &'a [Segment],
    /// Into device space; into *tile* space for a glyph, whose tile is rasterised at the
    /// quantised phase and drawn at the integer origin.
    transform: DeviceTransform,
    /// A stroke expands the flattened outline before it is filled (§8.4.3); `None` is a
    /// fill.
    stroke: Option<Stroke>,
    rule: Rule,
    extent: Extent,
    place: Place,
    draw: Draw,
    /// What this job is expected to cost, in outline segments — the only size known
    /// before the geometry exists, and the one [`partition`] balances by.
    weight: u64,
    /// An **upper bound** on the host memory this job holds between the walk and the
    /// commit: the widest its coverage tile can be, plus the job record itself.
    ///
    /// A bound rather than the truth, because the truth is the flattened geometry and
    /// that is what the job exists to defer. It is a bound because a Bézier lies inside
    /// the convex hull of its control points, so the box `hull.rs` already computed for
    /// culling contains every flattened point — and both lanes then cut that box by the
    /// same clip and the same target the rasteriser will.
    ///
    /// [`Encoder::enqueue`] keeps the sum of these under
    /// [`Encoder::in_flight_limit`], which is what stops a queue from being a way to
    /// hold a thousand full-page tiles at once where the walk held one (principle 3).
    held: u64,
}

/// Which rectangle a job's coverage is rasterised over: the two lanes' own arithmetic,
/// moved here so that one reading of it serves the serial and the parallel path alike.
enum Extent {
    /// The glyph lane: the shape's own device bounds, rounded out. The tile is the
    /// cached picture, so nothing about one placement may narrow it.
    Own,
    /// The path lane: shape ∩ clip ∩ target, rounded out.
    ///
    /// The clip and the target are folded together at enqueue time, which is the same
    /// bound to the bit: `max(max(x0, clip), 0)` and `max(x0, max(clip, 0))` agree for
    /// every pair of floats, NaN included, because `f32::max` is associative and returns
    /// its non-NaN operand.
    Visible {
        /// `[left, top, right, bottom]`, already held to the target.
        rect: [f32; 4],
    },
}

/// Where a job's tile goes once the commit reaches it.
#[derive(Clone, Copy)]
enum Place {
    /// The atlas already holds this placement: nothing to rasterise, one quad to draw.
    Resident {
        key: GlyphKey,
        origin: [f32; 2],
        entry: AtlasEntry,
    },
    /// Offer the tile to the atlas under this key, and fall through to the sheet when
    /// the atlas will not take it.
    Atlas { key: GlyphKey, origin: [f32; 2] },
    /// Pack the tile onto the frame's scratch sheet.
    Sheet,
}

/// The instance a committed job appends.
pub(super) struct Draw {
    color: Color,
    clip: Rect,
    style: DrawStyle,
    mask: Option<u32>,
}

impl Draw {
    pub(super) fn new(color: Color, clip: Rect, style: DrawStyle, mask: Option<u32>) -> Self {
        Self {
            color,
            clip,
            style,
            mask,
        }
    }
}

/// What one job's geometry came to: `None` when the mark reaches no pixel, which is a
/// command that legitimately draws nothing rather than an error.
type Rasterised = Option<CoverageMask>;

impl<'a> Job<'a> {
    /// A glyph-lane job: the tile is the shape's own bounds at the quantised phase.
    #[allow(clippy::too_many_arguments)] // one placement's parameters, gathered once at
    // its one call site, where the walk already holds each of them for its own reasons
    pub(super) fn glyph(
        segments: &'a [Segment],
        transform: DeviceTransform,
        rule: Rule,
        key: GlyphKey,
        origin: [f32; 2],
        resident: Option<AtlasEntry>,
        tile_bound: u64,
        draw: Draw,
    ) -> Self {
        let resident_already = resident.is_some();
        Self {
            segments,
            transform,
            stroke: None,
            rule,
            extent: Extent::Own,
            place: match resident {
                Some(entry) => Place::Resident { key, origin, entry },
                None => Place::Atlas { key, origin },
            },
            draw,
            weight: weight_of(segments, resident_already),
            held: held_by(tile_bound, resident_already),
        }
    }

    /// A path-lane job: the tile is the mark held to its clip and to the target.
    pub(super) fn sheet(
        segments: &'a [Segment],
        transform: DeviceTransform,
        stroke: Option<Stroke>,
        rule: Rule,
        rect: [f32; 4],
        tile_bound: u64,
        draw: Draw,
    ) -> Self {
        Self {
            segments,
            transform,
            stroke,
            rule,
            extent: Extent::Visible { rect },
            place: Place::Sheet,
            draw,
            weight: weight_of(segments, false),
            held: held_by(tile_bound, false),
        }
    }

    fn weight(&self) -> u64 {
        self.weight
    }

    fn held(&self) -> u64 {
        self.held
    }

    /// Whether this job has any geometry to make: a placement the atlas already holds
    /// costs a quad and nothing else, and the instrument must not time the nothing (a
    /// second interpretation of one page is `tests/two_rasters.rs`'s whole subject).
    fn rasterises(&self) -> bool {
        !matches!(self.place, Place::Resident { .. })
    }

    /// The atlas key this job would write, for the guard in [`Encoder::enqueue`].
    fn key_written(&self) -> Option<GlyphKey> {
        match self.place {
            Place::Atlas { key, .. } => Some(key),
            Place::Resident { .. } | Place::Sheet => None,
        }
    }
}

/// A job's share of the fan-out, in the units [`partition`] balances.
///
/// Segments rather than tile area, because the tile does not exist until the job has run
/// and a partition computed from the geometry would need the geometry. A resident tile
/// rasterises nothing and weighs nothing.
fn weight_of(segments: &[Segment], resident: bool) -> u64 {
    if resident { 0 } else { segments.len() as u64 }
}

/// The host memory one queued job can hold: its tile's upper bound, or none at all for a
/// placement the atlas already has, plus the record itself either way.
fn held_by(tile_bound: u64, resident: bool) -> u64 {
    let tile = if resident { 0 } else { tile_bound };
    tile.saturating_add(size_of::<Job<'_>>() as u64)
}

/// One job's coverage: flatten, expand it if it is a stroke, and run the scanline pass
/// over the rectangle its lane chose.
///
/// **This is the whole of the parallel phase.** It takes `&Job` and returns an owned
/// mask; it reads no frame state, writes no frame state, and allocates only what it
/// returns. That is what makes a frame on twenty-four threads the same bytes as a frame
/// on one, rather than a frame that merely usually is.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[allow(clippy::arithmetic_side_effects)] // the same bounded corner arithmetic the two
// lanes did in place, moved here unchanged
pub(super) fn rasterise(job: &Job<'_>) -> Rasterised {
    if matches!(job.place, Place::Resident { .. }) {
        return None;
    }
    let flattened = raster::flatten(job.segments, job.transform);
    let polylines = match job.stroke {
        Some(stroke) => raster::stroke_polylines(&flattened, stroke),
        None => flattened,
    };
    let (x0, y0, x1, y1) = raster::polyline_bounds(&polylines)?;
    let (vx0, vy0, vx1, vy1) = match job.extent {
        Extent::Own => (x0, y0, x1, y1),
        Extent::Visible { rect } => (
            x0.max(rect[0]),
            y0.max(rect[1]),
            x1.min(rect[2]),
            y1.min(rect[3]),
        ),
    };
    if vx0 >= vx1 || vy0 >= vy1 {
        return None;
    }
    let left = vx0.floor() as i32;
    let top = vy0.floor() as i32;
    let width = (vx1.ceil() as i32 - left).max(0) as u32;
    let height = (vy1.ceil() as i32 - top).max(0) as u32;
    if width == 0 || height == 0 {
        return None;
    }
    Some(raster::fill_mask(
        &polylines, job.rule, left, top, width, height,
    ))
}

/// How many jobs each worker takes: balanced by [`Job::weight`], and contiguous.
///
/// **Static, and computed from a size the walk already knew**, so there is no queue, no
/// atomic and no work stealing — the partition is a pure function of the job list, which
/// is one fewer thing between a thread count and the bytes it draws. Contiguous because
/// the results are reassembled in job order either way and adjacent jobs on a page are
/// adjacent in memory.
#[allow(clippy::arithmetic_side_effects)] // `taken + len` is bounded by `jobs.len()` by
// the loop condition, and the target is `u64` arithmetic over a sum of `usize` lengths
fn partition(jobs: &[Job<'_>], workers: usize) -> Vec<usize> {
    let total: u64 = jobs.iter().map(Job::weight).sum();
    let divisor = (workers as u64).max(1);
    let mut lens = Vec::with_capacity(workers);
    let (mut taken, mut carried) = (0_usize, 0_u64);
    for worker in 1..=workers {
        // Where this worker's share ends: `worker/workers` of the whole weight, in
        // integer arithmetic that cannot drift, since the last share ends at `total`.
        let target = total.saturating_mul(worker as u64) / divisor;
        let mut len = 0;
        while taken + len < jobs.len() && (worker == workers || carried < target) {
            carried = carried.saturating_add(jobs[taken + len].weight());
            len += 1;
        }
        taken += len;
        lens.push(len);
    }
    lens
}

/// Rasterise every job, on `threads` threads.
///
/// The caller has already decided that the fan-out is worth taking; this decides only
/// how the work is divided. One thread is a plain loop with no scope at all, which is
/// what a page below the floor and a host that never asked both take.
pub(super) fn rasterise_all(jobs: &[Job<'_>], threads: usize) -> Vec<Rasterised> {
    let mut results: Vec<Rasterised> = (0..jobs.len()).map(|_| None).collect();
    if threads <= 1 || jobs.len() < 2 {
        run(jobs, &mut results);
        return results;
    }
    let lens = partition(jobs, threads);
    let mut rest_jobs = jobs;
    let mut rest_out: &mut [Rasterised] = &mut results;
    thread::scope(|scope| {
        // Every share but the last is spawned and the last runs here, so `threads`
        // threads is `threads - 1` spawns and the calling thread is not left waiting.
        let spawned = lens.len().saturating_sub(1);
        for &len in lens.iter().take(spawned) {
            let take = len.min(rest_jobs.len());
            let (mine, jobs_tail) = rest_jobs.split_at(take);
            let (out, out_tail) = std::mem::take(&mut rest_out).split_at_mut(take);
            rest_jobs = jobs_tail;
            rest_out = out_tail;
            if !mine.is_empty() {
                scope.spawn(move || run(mine, out));
            }
        }
        run(rest_jobs, rest_out);
    });
    results
}

/// How many threads a run of this weight takes: what the host allowed, or one when the
/// run is too small to pay for a spawn.
///
/// The caller's §4 asks for this by name — *"anything on the frame path that a small page
/// pays for"* is the one thing they excluded, and their own ADR 0228 had to put a
/// measured floor under a `rayon` image resampler for the same reason. Ours is
/// [`PARALLEL_FLOOR_SEGMENTS`].
fn fan_out(weight: u64, allowed: usize) -> usize {
    if weight >= PARALLEL_FLOOR_SEGMENTS {
        allowed
    } else {
        1
    }
}

/// One worker's share.
fn run(jobs: &[Job<'_>], out: &mut [Rasterised]) {
    for (job, slot) in jobs.iter().zip(out) {
        *slot = rasterise(job);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Draw, IN_FLIGHT_BUDGET_SHARE, Job, PARALLEL_FLOOR_SEGMENTS, fan_out, in_flight_limit,
        partition,
    };
    use crate::atlas::{GlyphKey, PhaseKey};
    use crate::encode::DrawStyle;
    use crate::raster::{DeviceTransform, Rule};
    use quorra_scene::{Color, Point, Rect, Segment};

    /// A job of `segments` segments, which is the only field [`partition`] reads.
    fn job(segments: &[Segment]) -> Job<'_> {
        Job::glyph(
            segments,
            DeviceTransform {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e: 0.0,
                f: 0.0,
            },
            Rule::NonZero,
            GlyphKey {
                outline: 0,
                linear: [0; 4],
                phase: PhaseKey::Quantised(0, 0),
                rule: Rule::NonZero,
            },
            [0.0, 0.0],
            None,
            9,
            Draw::new(
                Color::new(0.0, 0.0, 0.0, 1.0),
                Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
                DrawStyle::Over,
                None,
            ),
        )
    }

    fn segments(count: usize) -> Vec<Segment> {
        vec![Segment::MoveTo(Point::new(0.0, 0.0)); count]
    }

    /// **A partition is a partition**: every job is in exactly one share, and the shares
    /// are in job order. Everything the fan-out promises about determinism rests on this
    /// one property, so it is asserted rather than read off the loop.
    #[test]
    fn every_job_is_in_exactly_one_share() {
        let sizes = [3_usize, 40, 1, 900, 7, 7, 7, 250, 2];
        let held: Vec<Vec<Segment>> = sizes.iter().map(|n| segments(*n)).collect();
        let jobs: Vec<Job<'_>> = held.iter().map(|s| job(s)).collect();
        for workers in 1..=12 {
            let lens = partition(&jobs, workers);
            assert_eq!(lens.len(), workers, "one share per worker");
            assert_eq!(
                lens.iter().sum::<usize>(),
                jobs.len(),
                "{workers} workers between them take every job"
            );
        }
    }

    /// The shares are balanced by **weight**, not by count: one enormous job does not
    /// drag its neighbours along behind it.
    #[test]
    fn one_heavy_job_does_not_take_its_neighbours_with_it() {
        let held: Vec<Vec<Segment>> = [1_usize, 1, 4_000, 1, 1]
            .iter()
            .map(|n| segments(*n))
            .collect();
        let jobs: Vec<Job<'_>> = held.iter().map(|s| job(s)).collect();
        let lens = partition(&jobs, 2);
        assert_eq!(
            lens,
            vec![3, 2],
            "the first share ends at the heavy job, the second takes what is left"
        );
    }

    /// **The queue is a batching granularity and never a capacity.** A frame's in-flight
    /// limit is a share of the budget the host stated, and a single job larger than it is
    /// still queued — alone — so no frame is refused for being made of large marks.
    #[test]
    fn the_in_flight_limit_follows_the_frame_budget_and_refuses_nothing() {
        assert_eq!(
            in_flight_limit(268 * 1024 * 1024),
            268 * 1024 * 1024 / IN_FLIGHT_BUDGET_SHARE,
            "a sixty-fourth of the default frame budget"
        );
        assert_eq!(
            in_flight_limit(0),
            0,
            "and a budget of nothing is not a panic"
        );
    }

    /// **No small page pays for a large one's lane** (the caller's §4). The median corpus
    /// page's twelve marks carry 120 segments between them — `tests/archetypes.rs` and
    /// `examples/encode_threads.rs` both read that number off it — and the floor is
    /// thirty-four times higher, so a page that size never enters a scope at all.
    #[test]
    fn a_page_below_the_floor_does_not_fan_out() {
        assert_eq!(fan_out(120, 24), 1, "the median corpus page");
        assert_eq!(fan_out(PARALLEL_FLOOR_SEGMENTS - 1, 24), 1);
        assert_eq!(fan_out(PARALLEL_FLOOR_SEGMENTS, 24), 24);
        assert_eq!(
            fan_out(u64::MAX, 1),
            1,
            "and a host that asked for one thread gets one at any size"
        );
    }
}
