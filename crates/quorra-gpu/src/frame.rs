//! The frame, and the measurements that make it accountable.
//!
//! The caller gates performance in CI and attributes regressions by measurement. §8 of
//! the brief is blunt about what that requires, and §6.1 exists *because* the current
//! backend could not answer it: the readback could not be separated from the execution,
//! so a bytes-per-second estimate had to stand in for what a timestamp query would have
//! told them exactly.
//!
//! # The rule this module exists to enforce
//!
//! **A failed frame must not be reported as a drawn one.** A [`Frame`] is constructed
//! only after every fallible step of its render has succeeded; every earlier exit is a
//! [`RenderError`]. And a [`Timings`] whose `execute` is a wall clock rather than a
//! timestamp query says which it is ([`TimingProvenance`]), because a number whose
//! provenance is ambiguous cannot gate anything — wall clocks lie under load: the mean
//! of ten frames put one of §6.1's figures at 15 ms where the fastest of ten put it
//! at 12.
//!
//! [`RenderError`]: crate::error::RenderError

use std::time::Duration;

use crate::error::RenderError;
use crate::report::Report;

/// Straight-alpha RGBA8 pixels, row-major, top row first, no padding — the caller's
/// `Raster` shape and what PNG expects (§3 of the brief).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raster {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Raster {
    /// A raster from its parts. Internal: rasters are produced by rendering, and the
    /// length invariant (`width × height × 4`) is established by the readback path.
    pub(crate) fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        debug_assert_eq!(
            pixels.len() as u64,
            u64::from(width)
                .saturating_mul(u64::from(height))
                .saturating_mul(4)
        );
        Self {
            width,
            height,
            pixels,
        }
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixel bytes: straight-alpha RGBA8, row-major, top row first, no padding.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Consume the raster and take its bytes.
    #[must_use]
    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }
}

/// Where a duration came from, because the two sources do not lie in the same ways.
///
/// Wall clocks lie under load; timestamp queries measure device time and do not. A
/// gate may trust `TimestampQueries`; a `WallClock` number is context, not evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimingProvenance {
    /// Measured on the device with timestamp queries.
    TimestampQueries,
    /// The adapter offers no timestamp queries; the number is a host-side wall clock
    /// over the same span, and says so rather than pretending otherwise.
    WallClock,
}

/// What this frame cost, phase by phase (§8 of the brief).
#[derive(Debug, Clone)]
pub struct Timings {
    /// CPU: turning a [`Scene`](quorra_scene::Scene) into device commands.
    pub encode: Duration,
    /// CPU: preparing and scheduling CPU→GPU transfers for this frame.
    pub upload: Duration,
    /// Device time for the drawing passes, from timestamp queries where available —
    /// see [`execute_provenance`](Timings::execute_provenance).
    pub execute: Duration,
    /// GPU→CPU: copy-out, map, and the premultiplied→straight conversion, as a wall
    /// clock (the span is CPU-bound and includes a wait, which is exactly what the
    /// caller pays). Zero for `Surface` and `Texture` targets.
    pub readback: Duration,
    /// Whether [`execute`](Timings::execute) came from timestamp queries or had to be
    /// a wall clock.
    pub execute_provenance: TimingProvenance,
    /// Per-pass device durations, present when timestamp queries are. M1 has one pass;
    /// entries accumulate as milestones add passes. May also carry one-off costs a
    /// frame absorbed, such as a first-use pipeline compilation.
    pub phases: Vec<(&'static str, Duration)>,
}

/// What this frame did, in counts (§8 of the brief).
///
/// The atlas and tiling counters exist from M1 with honest zeros: the fields are the
/// contract, and the milestones that build those subsystems start filling them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// Scene commands encoded into this frame.
    pub commands: u32,
    /// Distinct outlines referenced (M2 onwards; 0 until then).
    pub distinct_outlines: u32,
    /// Entries resident in the glyph atlas after this frame (M4; 0 until then).
    pub atlas_entries: u32,
    /// §6.3 of the brief: the count of **distinct keys** this frame asked the atlas
    /// for — deliberately not a hit rate. A hit rate is a statement about the lookups
    /// you made, never about the ones you should have made: a clip-mask cache once
    /// answered all 303 lookups a page made and built 303 identical page-wide masks,
    /// because the key was a name rather than the region.
    pub atlas_distinct_keys: u32,
    /// §6.4's instrument: how many **distinct clip regions** this frame resolved,
    /// after chains collapsed to device-space rectangles. Deliberately not a hit
    /// rate, and keyed by the resolved region rather than by identifier — the
    /// caller's clip-mask cache once answered all 303 lookups a page made and built
    /// 303 identical page-wide masks, because its key was a name (its ADR 0132; the
    /// same page collapses to 1 here).
    pub clip_distinct_regions: u32,
    /// Tiles touched by the general path lane (M5; 0 until then).
    pub tiles: u32,
    /// Path segments processed (M5; 0 until then).
    pub segments: u32,
    /// Bytes scheduled for CPU→GPU transfer this frame.
    pub bytes_uploaded: u64,
}

/// A drawn frame: what it cost, what it counted, and what it could not do as asked.
///
/// Everything a `Frame` says about itself is true. It exists only if the render
/// succeeded; a refused frame is a [`RenderError`], never a `Frame` that looks drawn.
/// A frame over a blank scene is a legitimate frame: zero commands, no reports, `Ok`.
#[derive(Debug)]
pub struct Frame {
    pub(crate) timings: Timings,
    pub(crate) counters: Counters,
    pub(crate) reports: Vec<Report>,
    pub(crate) payload: Payload,
}

/// What the render call produced besides accounting. Private: which payload a frame
/// carries is fully determined by the `Target` the caller passed.
#[derive(Debug)]
pub(crate) enum Payload {
    /// `Surface` and `Texture` targets: the pixels are where the caller asked.
    None,
    /// `Readback`: the pixels, held until [`Frame::into_raster`].
    Raster(Raster),
}

impl Frame {
    /// Everything the device could not draw as asked. Empty on a fully-honoured frame.
    #[must_use]
    pub fn reports(&self) -> &[Report] {
        &self.reports
    }

    /// What this frame cost, phase by phase.
    #[must_use]
    pub fn timings(&self) -> &Timings {
        &self.timings
    }

    /// What this frame did, in counts.
    #[must_use]
    pub fn counters(&self) -> Counters {
        self.counters
    }

    /// Take the pixels of a `Readback` frame.
    ///
    /// The readback itself — the expensive part — already happened inside `render` and
    /// was priced in [`Timings::readback`]; this is a move.
    ///
    /// # Errors
    ///
    /// [`RenderError::NotAReadbackFrame`] if the frame was rendered to a `Surface` or
    /// `Texture` target: those pixels are already where the caller asked, and
    /// pretending otherwise here would misreport what the frame did.
    pub fn into_raster(self) -> Result<Raster, RenderError> {
        match self.payload {
            Payload::Raster(raster) => Ok(raster),
            Payload::None => Err(RenderError::NotAReadbackFrame),
        }
    }
}
