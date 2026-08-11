//! What `encode` spent its time on, when a caller asks it to say.
//!
//! The caller's feedback §13 measured `Device::render` and found `encode` the largest
//! of its three phases and the only one that tracks the scene's size — 3.86 µs a
//! command by least squares over 38 page turns — and then said the thing that makes
//! this module rather than an optimisation: *"whether those 3.86 µs a command are path
//! flattening, bind-group churn, buffer writes, sorting, or `wgpu`'s own command
//! recording is invisible from here"*. An instrument before an optimisation, which is
//! the argument the three phases already won.
//!
//! # Why it is a switch and not always on
//!
//! The parts of encode interleave per command — a fill flattens, rasterises, packs and
//! records, then the next one does — so subdividing means a clock read at each seam
//! rather than three for the frame. `Instant::now()` is about 20 ns here, and a page of
//! 5 933 commands crosses those seams often enough to cost ~0.2 ms: three times the
//! whole encode of a page of rectangles (0.089 ms), and 1% of a page of paths. A
//! measurement that changes the number it measures by 300% is not an instrument, so
//! this is [`Options::instrument_encode`](crate::startup::Options::instrument_encode),
//! off by default, and it costs a disabled `Option` check when it is off.
//!
//! # The parts
//!
//! - **geometry** — flattening outlines to polylines, expanding strokes, and running
//!   the scanline rasteriser: making coverage out of shapes.
//! - **staging** — packing that coverage into the frame's scratch sheet and the glyph
//!   atlas: the memory traffic that carries it, as distinct from the arithmetic that
//!   produced it.
//! - **recording** — the remainder, computed rather than measured: clip resolution,
//!   culling, instance building, plan assembly.
//!
//! Three parts because the caller asked for "two or three", and because a seam that
//! cannot be named is a seam nobody can act on.

use std::cell::Cell;
use std::time::{Duration, Instant};

/// Accumulated encode sub-phases. Interior mutability on purpose: the seams sit where a
/// resource is already borrowed from the encoder, and an instrument that forced the code
/// around it to change shape would be measuring something else.
#[derive(Debug, Default)]
pub(crate) struct EncodeClock {
    enabled: bool,
    geometry: Cell<Duration>,
    staging: Cell<Duration>,
}

impl EncodeClock {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            geometry: Cell::new(Duration::ZERO),
            staging: Cell::new(Duration::ZERO),
        }
    }

    /// Open a span, or nothing at all when the switch is off.
    pub(crate) fn start(&self) -> Option<Instant> {
        self.enabled.then(Instant::now)
    }

    /// Close a span into the geometry total.
    pub(crate) fn geometry(&self, started: Option<Instant>) {
        Self::add(&self.geometry, started);
    }

    /// Close a span into the staging total.
    pub(crate) fn staging(&self, started: Option<Instant>) {
        Self::add(&self.staging, started);
    }

    fn add(slot: &Cell<Duration>, started: Option<Instant>) {
        if let Some(started) = started {
            slot.set(slot.get().saturating_add(started.elapsed()));
        }
    }

    /// The split, as phases, given what the whole of encode cost.
    ///
    /// Empty when the switch was off, so a caller reading `Timings::phases` sees the
    /// subdivision exactly when it asked for one. `recording` is the remainder and is
    /// saturating: the two measured parts are spans inside the whole, but they are
    /// summed from many small spans and the whole is one large one, so on a clock
    /// coarse enough they could round past it.
    pub(crate) fn phases(&self, encode: Duration) -> Vec<(&'static str, Duration)> {
        if !self.enabled {
            return Vec::new();
        }
        let geometry = self.geometry.get();
        let staging = self.staging.get();
        let recording = encode.saturating_sub(geometry).saturating_sub(staging);
        vec![
            ("encode: geometry", geometry),
            ("encode: staging", staging),
            ("encode: recording", recording),
        ]
    }
}
