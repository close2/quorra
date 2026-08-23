//! The probes: **a drawn raster turned into the number an assertion is written on.**
//!
//! One responsibility, and it is the mirror of [`super::headless`]'s. That module asks a
//! device for a frame and hands back its bytes; this one turns those bytes into the byte a
//! test names ([`pixel`], [`alpha`]). Nothing here draws, and nothing here asserts: the
//! expectation and its bound belong to the file that states them, because both are
//! properties of that file's fixture rather than of this arithmetic. [`super::clause`]
//! already splits along that seam and says so.
//!
//! # `max_byte_diff` was here and is not
//!
//! It reduced a whole raster to its largest per-byte difference, and it had three call
//! sites when `doc/notes-test-probes.md` gave the probes this home. ADR 0072 took two of
//! them and ADR 0077 the third, both for the same reason: a single number over a whole
//! raster has to be compared against a bound that holds at the raster's *worst* pixel, so
//! it hands out that slack at every other one — and it cannot express the clause that
//! matters most, which is that a pixel nothing stored to must agree **exactly**.
//! [`super::bound`] is what replaced it, and it is not a generalisation of this module: it
//! reads a store count the reference had to count, which no probe over two rasters can
//! recover. Deleted rather than kept for a future caller, because
//! `tests/common/mod.rs`'s `allow(dead_code)` means an unused helper here is one nothing
//! will ever notice.
//!
//! # Why a probe takes the raster's width, and why that is what made the merge safe
//!
//! `doc/HANDOVER.md` recorded these as deliberately *not* unified, and the recorded
//! obstacle was that each copy indexed through its own file's `SIZE` — so one home for the
//! probes looked like one home for `SIZE`, which means 64 in six files and something else
//! in four others. The premise did not survive being read: **a probe needs the raster's
//! stride, not the suite's `SIZE`**, and a stride is an argument.
//!
//! That distinction is the whole safety property. A probe that closes over a file-scoped
//! constant carries its dimension invisibly, which is why `alpha`'s text could be
//! identical in `coverage_lanes.rs` and `mask_regions.rs` while the `SIZE` it read was
//! not — a merge of those two texts would have returned a plausible byte from the wrong
//! place, in silence. A probe that takes the stride carries no dimension at all: there is
//! no `SIZE` here for it to read, and every caller names its own at the call site, where a
//! reviewer can see it beside the coordinates.
//!
//! The tree already held this answer in three places before the merge — `function_lane.rs`
//! and `m7.rs` took `width`, `thin_marks.rs` took `side` — because those three draw at more
//! than one size and so could not close over a constant. The merge is convergence on the
//! shape that was already there, not a new one.
//!
//! **The height is deliberately not a parameter.** An index into row-major RGBA is a
//! function of the stride alone; asking for a number the arithmetic never uses is a number
//! that can be wrong without anything failing. A raster's height is checked by the length
//! of the slice, which is what the index panics against.

/// The four straight-alpha RGBA bytes of the device pixel `(x, y)` of a raster `width`
/// pixels across.
///
/// Panics if the pixel is outside the raster, which is the intended behaviour: a probe
/// reading past its raster is a test asking about a pixel that was never drawn.
pub fn pixel(raster: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;
    [raster[at], raster[at + 1], raster[at + 2], raster[at + 3]]
}

/// The alpha of the device pixel `(x, y)` of a raster `width` pixels across.
///
/// Written as `pixel(..)[3]` rather than as its own index, so that the two probes cannot
/// come to disagree about which byte belongs to which pixel.
pub fn alpha(raster: &[u8], width: u32, x: u32, y: u32) -> u8 {
    pixel(raster, width, x, y)[3]
}
