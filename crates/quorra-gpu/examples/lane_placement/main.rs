//! **Where each coverage lane puts a hairline, and what the sampled grid does to its ink.**
//!
//! The caller's `QUORRA_FEEDBACK.md` §31 reports that our two coverage lanes disagree
//! about the *placement* of an axis-aligned rule about one device pixel wide, by up to an
//! eighth of a device pixel, on four of their corpus pages. They ask two questions, and
//! this is the instrument that answers them from our own arithmetic rather than from their
//! pages:
//!
//! 1. is the default (`Coverage::Cpu`) lane's per-command offset intended, and if it is a
//!    quantisation, what is its bound?
//! 2. is the `Coverage::Gpu` lane's y coverage quantised, and to what?
//!
//! # The four phases, and which question each is
//!
//! | phase | picture | what it reaches |
//! |---|---|---|
//! | 1 **placement** | one rule, swept through a whole pixel of position | both lanes' placement, question 1 and half of 2 |
//! | 2 **grid** | one rule of a width the sample lattice does not divide, at three sample counts | the sampled lane's *ink*, question 2 |
//! | 3 **graph paper** | six rules, six commands, at the caller's own pitch and CTM | question 1's per-command offset, which one swept rule cannot show |
//! | 4 **witness** | their published §31.2 table, put through this lane's own grid arithmetic | question 1, from the only six numbers of theirs we have |
//!
//! Phase 4 renders nothing. It is there because the conclusion it reaches — that their two
//! columns are one geometry, quantised — would otherwise be a paragraph in a note, and a
//! paragraph is not something a later round can watch fail.
//!
//! # What it measures
//!
//! For each position, per lane: the **centroid** of the rule's coverage along the axis it
//! is swept in, and the **total ink** in units of a fully covered pixel. Both are compared
//! against the exact values, which for a band of known width and position are arithmetic
//! and not another rasterisation. That is what makes this a check against the definition
//! (ADR 0005's area rule) rather than a comparison of two implementations.
//!
//! ```text
//! cargo run --release -p quorra-gpu --example lane_placement
//! ```
//!
//! `--check` runs the smallest sweep that still exercises every phase and both lanes.
//!
//! # Three traps this instrument has already fallen into, all one shape
//!
//! **A fixture whose parameter is a multiple of the grid under test measures that grid's
//! fixed points, and a fixed point looks exactly like conformance.** Three constants here
//! exist only because of it, and each was written after a run that reported nothing:
//!
//! - [`STEPS`] is a prime. Sixteen steps land every sample on the atlas's own 1/16
//!   quantisation grid, and the sweep reported zero error at all sixteen while a
//!   whole-pixel defect sat inside the buckets (`doc/notes-glyph-phase-carry.md` §2).
//! - [`CHECK_STEPS`] is a prime. Four steps are 0, ¼, ½ and ¾, every one a multiple of the
//!   sampled grid's own ¼ pitch, so phase 2's ladder came back with **one rung** where
//!   there are two.
//! - `fixture::WITNESS_WIDTH` is not a multiple of any sample pitch, because the trap moved
//!   axis: a band whose *width* the lattice period divides contains the same number of
//!   lattice points wherever it lands and loses no ink at any position at all. That is why
//!   phase 2 exists separately from phase 1 rather than reusing its width, and it is the
//!   same reason `tests/thin_marks.rs` gained `OFF_LATTICE_WIDTHS` in this round.

// The lint policy every example in this tree states: an example's arithmetic is bounded
// by a target it just allocated, and a proof that cannot run must fail loudly.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

mod fixture;
mod measure;
mod sweeps;
mod witness;

use quorra_gpu::{Coverage, DEFAULT_COVERAGE_SAMPLES, DEFAULT_GLYPH_QUANTUM, Device};

use fixture::{ACROSS, ALONG, HAIRLINE, TINY_ATLAS, WITNESS_WIDTH, device};
use sweeps::{
    BYTE_SLACK, CASES, CHECK_STEPS, GRAPH_WIDTHS, SAMPLE_COUNTS, STEPS, Worst, byte_drift,
    graph_paper_row, grid_sweep, placement_sweep,
};

/// Phase 1, and the two worsts it folds — the cpu column's and the sampled column's.
fn placement_phase(devices: &mut Devices, steps: u32) -> [Worst; 2] {
    let mut worst = [Worst::default(); 2];
    println!("--- phase 1: placement, a {HAIRLINE}-pixel rule through one pixel of position ---\n");
    for case in CASES {
        let lanes = if case.uncached {
            (&mut devices.cpu_uncached, &mut devices.gpu_uncached)
        } else {
            (&mut devices.cpu, &mut devices.gpu)
        };
        placement_sweep(lanes, case, steps, &mut worst);
    }
    worst
}

/// Phase 2, over every sample count — the answer to question 2.
fn grid_phase(devices: &mut Devices, steps: u32, counts: &[u32]) -> Vec<(u32, Vec<f64>, Worst)> {
    println!(
        "--- phase 2: the sampled grid's own quantum, at {} sample count(s) ---\n",
        counts.len()
    );
    let mut rows = Vec::new();
    for &samples in counts {
        let mut gpu = device(Coverage::Gpu, None, samples);
        let mut worst = Worst::default();
        let ladder = grid_sweep((&mut devices.cpu, &mut gpu), samples, steps, &mut worst);
        rows.push((samples, ladder, worst));
    }
    rows
}

/// The devices phases 1 and 3 draw on: two coverage settings, each with the default atlas
/// and with one too small to hold a rule's tile.
struct Devices {
    /// `Coverage::Cpu`, default atlas — the processor lane, and the glyph lane for a fill.
    cpu: Device,
    /// `Coverage::Gpu`, default atlas.
    gpu: Device,
    /// `Coverage::Cpu` on an atlas that refuses the tile, so a fill stays off the glyph lane.
    cpu_uncached: Device,
    /// `Coverage::Gpu` on an atlas that refuses the tile.
    gpu_uncached: Device,
}

fn main() {
    let check = std::env::args().any(|arg| arg == "--check");
    let steps = if check { CHECK_STEPS } else { STEPS };
    let counts: &[u32] = if check {
        &[DEFAULT_COVERAGE_SAMPLES]
    } else {
        &SAMPLE_COUNTS
    };
    let samples = DEFAULT_COVERAGE_SAMPLES;
    let mut devices = Devices {
        cpu: device(Coverage::Cpu, None, samples),
        gpu: device(Coverage::Gpu, None, samples),
        cpu_uncached: device(Coverage::Cpu, Some(TINY_ATLAS), samples),
        gpu_uncached: device(Coverage::Gpu, Some(TINY_ATLAS), samples),
    };
    println!("a rule on a {ALONG} × {ACROSS} target, {steps} positions per row");
    println!("centroid error is (lane − geometry) in device pixels; ink is in covered pixels\n");

    let placement = placement_phase(&mut devices, steps);
    let grid = grid_phase(&mut devices, steps, counts);
    println!("--- phase 3: the caller's graph paper, one command per rule ---\n");
    for width in GRAPH_WIDTHS {
        // Both axes, because the caller's two witnesses are one apiece: `bug1743245.pdf`
        // is measured along a raster row and `issue21068.pdf` is "the same statement in
        // the other axis" (§31.2).
        for horizontal in [true, false] {
            graph_paper_row((&mut devices.cpu, &mut devices.gpu), width, horizontal);
        }
    }
    witness::against_the_callers_table(samples);

    report(&placement, &grid);
    assert_placement(&placement, samples);
    assert_grid(&grid);
}

/// What the run came to, printed before anything is asserted so that a failure is read
/// beside its own numbers.
fn report(placement: &[Worst; 2], grid: &[(u32, Vec<f64>, Worst)]) {
    println!("--- what the sweeps came to ---\n");
    println!(
        "phase 1 worst centroid error: cpu column {:+.4}, sampled column {:+.4}",
        placement[0].centroid, placement[1].centroid
    );
    println!(
        "phase 1 worst ink error:      cpu column {:+.4}, sampled column {:+.4}",
        placement[0].ink, placement[1].ink
    );
    for (samples, ladder, worst) in grid {
        let pitch = 1.0 / f64::from(*samples).sqrt();
        println!(
            "phase 2 at {samples:>2} samples: {} rungs, worst ink {:+.4} and worst centroid \
             {:+.4}, against the pitch's own {pitch:.4}",
            ladder.len(),
            worst.ink,
            worst.centroid
        );
    }
    println!();
}

/// The bound the atlas's quantum states (ADR 0009, ADR 0073), and the bound the sample
/// grid states — asserted rather than printed, because between them they are the two
/// properties this instrument exists to hold.
fn assert_placement(worst: &[Worst; 2], samples: u32) {
    // **The cpu column, and the assertion this file has carried since ADR 0073.** A
    // placement is rounded to the nearest of `q` buckets, so it moves by at most half of
    // one. `Coverage::Cpu` never reaches the sampled lane, so this column is the atlas's
    // quantum and nothing else, and the glyph-lane row of phase 1 is what makes it able to
    // fail — the carry defect moved that row by a whole pixel.
    let quantum = 1.0 / (2.0 * f64::from(DEFAULT_GLYPH_QUANTUM));
    assert!(
        worst[0].centroid.abs() <= quantum + BYTE_SLACK,
        "the cpu column moved a hairline by {:+.4} device pixels, past the quantum's own \
         bound of {quantum:.4}",
        worst[0].centroid
    );
    // **The sampled column, and its bound is a different number for a different reason.**
    // The device lane counts samples on an ordered grid whose rows are `1/√n` apart
    // (`winding::sample_offsets`), so a band covers the lattice points inside it and its
    // centroid can only be their mean. **Half a pitch holds here because [`HAIRLINE`] is a
    // multiple of the pitch**: the band then contains the same count of lattice points
    // wherever it lands, the mean is the first point plus a fixed offset, and the first
    // point is within one pitch of the band's edge — so the mean is within half a pitch of
    // its centre. At a width the pitch does not divide the count itself changes and the
    // bound is a whole pitch, which is what `assert_grid` states and phase 2 measures.
    // This is not the atlas's quantum and it is four times larger at the default sample
    // count; ADR 0076 is the decision.
    let half_pitch = 0.5 / f64::from(samples).sqrt();
    assert!(
        worst[1].centroid.abs() <= half_pitch + BYTE_SLACK,
        "the sampled column moved a hairline by {:+.4} device pixels, past half the sample \
         grid's own pitch of {half_pitch:.4}",
        worst[1].centroid
    );
}

/// That the sampled lane was **reached**, and that what it did there is the grid's own
/// arithmetic and not a constant that happens to be a quarter.
fn assert_grid(grid: &[(u32, Vec<f64>, Worst)]) {
    for (samples, ladder, worst) in grid {
        let pitch = 1.0 / f64::from(*samples).sqrt();
        let slack = byte_drift(*samples) + BYTE_SLACK;
        // Reachability first, and it is the property this round added. A rule shorter than
        // `ALONG` fails `triangles_under_coverage`'s floor and is answered by the processor
        // under both settings, and every number below would then be the processor's — which
        // is exactly what the previous revision of this instrument measured without knowing
        // it. A processor-drawn band of this width is exact to a byte at every position, in
        // both its ink and its placement, so either being off by a quarter of a pitch is
        // the sampled lane and nothing else.
        assert!(
            worst.ink.abs().max(worst.centroid.abs()) > pitch / 4.0,
            "at {samples} samples the sampled lane's ink was out by only {:+.4} and its \
             placement by {:+.4}, which is not what a grid of pitch {pitch:.4} does to a \
             {WITNESS_WIDTH}-pixel band: the mark never reached that lane. Check \
             `fixture::ALONG` against the triangle floor, and `CHECK_STEPS` against the pitch.",
            worst.ink,
            worst.centroid
        );
        // **One pitch, not half of one, and the difference is a correction this round
        // made after the sweep contradicted it.** The ink is `k` sample rows of `pitch`
        // each, where `k` is the number of lattice points a half-open interval of the
        // band's width contains — either `⌊w/pitch⌋` or `⌈w/pitch⌉`. Those two are a whole
        // pitch apart, so the error reaches `pitch·⌊w/pitch⌋ − w` on one side, which for a
        // band just over a pitch is nearly a whole pitch. Measured at four samples: a
        // 0.878-pixel band drew 0.5020, which is −0.3760 and would have passed a
        // half-pitch bound only because sixteen samples is where the instrument first ran.
        assert!(
            worst.ink.abs() < pitch + slack,
            "at {samples} samples the sampled lane's ink was out by {:+.4}, past the grid's \
             own pitch of {pitch:.4}",
            worst.ink
        );
        // The placement follows from the same count: the band's first lattice point is
        // anywhere within a pitch of its lower edge and the mean sits `(k−1)/2` pitches
        // above it, so the centroid moves by at most `pitch/2` from the first term and
        // `(k·pitch − w)/2` from the second — under a pitch in all.
        assert!(
            worst.centroid.abs() < pitch + slack,
            "at {samples} samples the sampled lane moved a {WITNESS_WIDTH}-pixel band by \
             {:+.4}, past the grid's own pitch of {pitch:.4}",
            worst.centroid
        );
        // Every rung on a lattice of `1/√n`, which is the claim ADR 0076 records: the ink
        // is a count of sample rows and therefore a multiple of the pitch, never a
        // constant fitted to one sample count.
        for rung in ladder {
            let nearest = (rung / pitch).round() * pitch;
            assert!(
                (rung - nearest).abs() <= slack,
                "at {samples} samples the sampled lane produced an ink of {rung:.4}, which is \
                 {:.4} off the nearest multiple of the grid's pitch {pitch:.4} — the ladder is \
                 not the lattice",
                rung - nearest
            );
        }
    }
}
