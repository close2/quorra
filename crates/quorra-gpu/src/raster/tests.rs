//! What the three parts of [`raster`](super) are asked to produce, on shapes whose answer
//! is derivable by hand or by a clause — **one file per clause**, and the fixtures more
//! than one of them builds.
//!
//! # The seam, and why it is the parent's after all
//!
//! ADR 0061 put every one of these in a single file when `raster.rs` was split, so that the
//! split could be proved by an identical list of test names, and wrote down the cost:
//! 704 lines, past the smell, and dividing them a legitimate round of its own. This is that
//! round, and its whole visible effect is that every test gains one path segment.
//!
//! ADR 0061 also gave a second, weaker reason for keeping them together: that *the tests
//! often do not divide along the source's seams*, since almost every case here goes through
//! all three parts — a stroke's caps are measured by **filling** the polygons
//! [`stroke`](super::stroke) expands, and a circle's area is a statement about
//! [`flatten`](mod@super::flatten) read out of [`fill`](super::fill)'s bytes.
//!
//! That was measured against the wrong question. **A test is filed by the clause it makes a
//! statement about, not by the code path it runs through** — `fill_mask` is the instrument
//! almost all of them observe through, and an instrument is not a subject.
//! `each_cap_deposits_the_area_table_53_gives_it` reads coverage bytes and is a statement
//! about §8.4.3.3's Table 53, so it is [`stroke`](stroke)'s; nothing about it is a claim
//! concerning §8.5.3.3's two rules. Asked that way the division is clean, with no case left
//! over. ADR 0062 records the rule.
//!
//! The parent's own module comment is the evidence that the seam holds, because it already
//! assigns each of this code's three arithmetic defects to one part —
//! `stroke::direction`'s, `fill::accumulate_edge`'s and `fill::deposit_slab`'s — and each
//! defect's test lands in its defect's file without anyone choosing that.
//!
//! | Module | The clause its cases are statements about |
//! |---|---|
//! | [`flatten`](flatten) | §10.7.2 and ADR 0044 — how finely a curve becomes chords, and what that costs its ink |
//! | [`fill`](fill) | §8.5.3.3 and ADR 0005/0049 — coverage from polylines, the two rules, and a region's cut at a tile border |
//! | [`stroke`](stroke) | §8.4.3 — caps, joins, and the expansion's arithmetic at the ends of the coordinate range |
//!
//! What stays here is what more than one of them builds: the identity transform every case
//! rasterises under, the coverage probe, and the rectangle path. Everything used by exactly
//! one file lives in that file, where its reasons are.
//!
//! # The one thing that changed besides the names
//!
//! The imports. ADR 0061 cost 3 predicted this and it is again the only other edit: these
//! files sit one level further from `raster`, so they name it absolutely
//! (`use crate::raster::…`) rather than counting `super`s, and reach `FLATTEN_TOLERANCE`
//! and `cubic_tolerance` through the private module that owns them — nothing outside
//! `raster` asks for either, so re-exporting them from the parent would widen them for
//! this directory's benefit alone.
#![allow(clippy::arithmetic_side_effects)] // test indices are tiny and literal

mod fill;
mod flatten;
mod stroke;

use quorra_scene::{Point, Segment};

use super::{CoverageMask, DeviceTransform};

const IDENTITY: DeviceTransform = DeviceTransform {
    a: 1.0,
    b: 0.0,
    c: 0.0,
    d: 1.0,
    e: 0.0,
    f: 0.0,
};

fn cov(mask: &CoverageMask, x: usize, y: usize) -> u8 {
    mask.coverage[y * mask.width as usize + x]
}

fn rect_path(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<Segment> {
    vec![
        Segment::MoveTo(Point::new(x0, y0)),
        Segment::LineTo(Point::new(x1, y0)),
        Segment::LineTo(Point::new(x1, y1)),
        Segment::LineTo(Point::new(x0, y1)),
        Segment::Close,
    ]
}
