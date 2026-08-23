//! **The sweeps: what is drawn, at which positions, and what comes back.**
//!
//! One responsibility, and the seam with `main` is the one CLAUDE.md's file rule points at:
//! everything here **measures** and prints, and nothing here asserts. The bounds those
//! measurements are held to live in `main`, so that a reader checking a claim reads one file
//! and a reader checking a number reads the other.
//!
//! Three sweeps — one per question the caller's `QUORRA_FEEDBACK.md` §31 asks, and one for
//! the construction their §31.2 is stated in. [`Worst`] is what each folds into and the only
//! thing that crosses back.

use quorra_gpu::{DEFAULT_COVERAGE_SAMPLES, DEFAULT_GLYPH_QUANTUM, Device};

use crate::fixture::{self, CTM, Case, HAIRLINE, WITNESS_WIDTH, graph_paper};
use crate::measure::{Reading, Window, one_rule, read, read_scene, row_split, unpainted_lanes};

/// What a byte of alpha is worth as a centroid error on this fixture: a coverage rounded
/// to 1/255 moves the measured centre by about this much, and it is not any quantiser's
/// doing. Measured at 0.0019 across the whole sweep with the quantum's own error at zero.
pub(crate) const BYTE_SLACK: f64 = 0.004;

/// How far the sampled lane's coverage can drift from the sample grid's own ladder, in
/// units of a fully covered pixel, at `samples` samples.
///
/// **Because the byte is written once per *group of four*, not once per frame.** The
/// winding pass holds four samples in an `rgba16float` texel, so a frame of `n` samples
/// runs the resolve pass `n/4` times and each of those adds `covered/n` into the
/// `r8unorm` sheet with additive blending — one rounding to 1/255 apiece. `winding.wgsl`'s
/// own comment on `fs_resolve` says "the single quantisation to a byte happens once, at
/// the store", and that is true of a four-sample frame and of no other. The bound is half
/// a level per round.
///
/// Measured against it: a full pixel of coverage reads 1.0039 at sixteen samples, which is
/// exactly one level over, against the four roundings' worst case of two.
pub(crate) fn byte_drift(samples: u32) -> f64 {
    f64::from(samples / 4) / 510.0
}

/// Sub-pixel positions swept across one whole pixel.
///
/// **A prime, and that is the point.** The atlas quantises a phase to
/// `Options::glyph_quantum` of a pixel (default 1/16) and the device lane's samples lie on
/// a lattice of 1/4, so a sweep of 16 steps lands every sample exactly on both grids and
/// measures zero error at every one of them — which is what the first run of this
/// instrument did. A step that shares no factor with either visits the whole of each
/// bucket instead.
pub(crate) const STEPS: u32 = 37;

/// The sample counts phase 2 sweeps, which are the squares `Options::coverage_samples`
/// admits at both ends of its clamp and in the middle.
///
/// **Three rather than one, because one is a curve fit.** A single count can only say
/// "the ink came out in quarters"; three say that the step is `1/√n` and therefore that it
/// is the grid `winding::sample_offsets` lays down, which is a statement about the code.
pub(crate) const SAMPLE_COUNTS: [u32; 3] = [4, DEFAULT_COVERAGE_SAMPLES, 64];

/// The pitch the caller's `bug1743245.pdf` states, in device pixels: their §31.2 gives the
/// document's own arithmetic as `52.0277778 × 0.317180616 = 16.5013`.
///
/// The user-space figure is written to the precision `f32` can hold; the document's own is
/// 52.0277778, and the two are the same `f32`.
const GRAPH_PITCH: f32 = 52.027_78 * CTM;

/// Where the caller's first graph-paper rule lands, in device pixels (their §31.2's oracle
/// column).
const GRAPH_FIRST: f32 = 33.0;

/// How many of their rules fit across [`ACROSS`] at [`GRAPH_PITCH`] — six, which is
/// exactly the run their table prints.
const GRAPH_RULES: u32 = 6;

/// The two device widths phase 3 draws the caller's graph paper at.
///
/// The first is their sentence's own arithmetic — "0.5-unit strokes under a 0.317 CTM",
/// which §4.5 of the brief settles upstream and hands us already resolved into device
/// pixels. The second is their §31.2's description of the population, "about one device
/// pixel wide". The two are a factor of six apart and only one of them can be what those
/// pages draw; printing both is how this instrument says which.
pub(crate) const GRAPH_WIDTHS: [f32; 2] = [0.5 * CTM, HAIRLINE];

/// The worst error one column of a sweep reached, folded as the sweep runs.
#[derive(Default, Clone, Copy)]
pub(crate) struct Worst {
    /// The signed centroid error of the largest magnitude, in device pixels.
    pub(crate) centroid: f64,
    /// The signed ink error of the largest magnitude, in covered pixels.
    pub(crate) ink: f64,
}

impl Worst {
    /// Folds one reading in, keeping whichever error is larger in magnitude.
    fn fold(&mut self, centroid: f64, ink: f64) {
        if centroid.abs() > self.centroid.abs() {
            self.centroid = centroid;
        }
        if ink.abs() > self.ink.abs() {
            self.ink = ink;
        }
    }
}

/// How many positions `--check` visits.
///
/// **A prime, for the same reason [`STEPS`] is, and it was not one until 2026-08-23.**
/// `--check` swept four steps — 0, ¼, ½, ¾ — every one of them a multiple of the sample
/// grid's own ¼ pitch, so the band's edges stood in exactly the same relation to the
/// lattice at all four and phase 2's ladder came back with **one rung**. The run looked
/// like a measurement and was the quantiser's fixed points, which is the trap
/// `doc/notes-glyph-phase-carry.md` §2 records having already been paid for once. Seven
/// shares no factor with 2, 4, 8 or 16, which is every grid this instrument sweeps
/// against.
pub(crate) const CHECK_STEPS: u32 = 7;

/// The sub-pixel positions one row visits: a uniform sweep, **plus the top of the last
/// quantum bucket**.
///
/// That extra position is not decoration and it is not negotiable. The only phases the
/// carry of ADR 0073 touches are `fx ≥ 1 − 1/2q` — 3.1 % of the pixel at the default
/// quantum — and a short uniform sweep need visit none of it. The assertion in `main`
/// therefore *could not fail for its own regression* under `--check` until this function
/// existed, which is ADR 0052's rule and was checked by forcing the defect and watching
/// `--check` stay green.
fn offsets(steps: u32) -> Vec<f32> {
    let quantum = f32::from(DEFAULT_GLYPH_QUANTUM);
    let mut positions: Vec<f32> = (0..steps)
        .map(|step| f32::from(step as u16) / steps as f32)
        .collect();
    // Inside the top half-bucket, where `(fx · q).round()` reaches `q` itself.
    positions.push(1.0 - 1.0 / (4.0 * quantum));
    positions
}

/// How one row of the placement sweep says what it is.
fn describe(case: Case) -> String {
    format!(
        "== {} rule, swept in {}, stated {}, {} ==",
        if case.horizontal {
            "horizontal"
        } else {
            "vertical"
        },
        if case.horizontal { "y" } else { "x" },
        if case.through_transform {
            "by the command's affine (a PDF's `cm`)"
        } else {
            "in the outline's own coordinates"
        },
        match (case.filled, case.uncached, case.repeated) {
            (true, true, _) => "as a fill the atlas refuses",
            (true, false, _) => "as a fill of its own band",
            (false, _, true) => "as a stroke, drawn twice (the atlas can hold it)",
            (false, _, false) => "as a stroke, drawn once (single-use: no atlas)",
        }
    )
}

/// The four rows of the placement sweep, in the order that isolates one thing at a time: a
/// stroke, the same rule as a fill (which the atlas takes under `Coverage::Cpu`), the same
/// fill with the atlas refusing it, and that again in the other axis.
pub(crate) const CASES: [Case; 4] = [
    Case {
        horizontal: true,
        through_transform: true,
        repeated: false,
        filled: false,
        uncached: false,
        width: HAIRLINE,
    },
    Case {
        horizontal: true,
        through_transform: true,
        repeated: false,
        filled: true,
        uncached: false,
        width: HAIRLINE,
    },
    Case {
        horizontal: true,
        through_transform: true,
        repeated: false,
        filled: true,
        uncached: true,
        width: HAIRLINE,
    },
    Case {
        horizontal: false,
        through_transform: true,
        repeated: false,
        filled: true,
        uncached: true,
        width: HAIRLINE,
    },
];

/// One row of phase 1: both settings at every sub-pixel position, printed, and the worst
/// error each column reached folded into `worst`.
pub(crate) fn placement_sweep(
    lanes: (&mut Device, &mut Device),
    case: Case,
    steps: u32,
    worst: &mut [Worst; 2],
) {
    let (cpu, gpu) = lanes;
    println!("{}", describe(case));
    println!(
        "{:>8}  {:>10}  {:>10} {:>9} {:>6}  {:>10} {:>9} {:>6}",
        "offset", "geometry", "cpu lane", "error", "lane", "gpu lane", "error", "lane"
    );
    for offset in offsets(steps) {
        let centre = fixture::BASE + offset;
        let (one, two) = (read(cpu, centre, case), read(gpu, centre, case));
        let exact = f64::from(centre);
        for (index, reading) in [&one, &two].into_iter().enumerate() {
            worst[index].fold(
                reading.centroid - exact,
                reading.ink - f64::from(case.width),
            );
        }
        println!(
            "{offset:>8.4}  {exact:>10.4}  {:>10.4} {:>+9.4} {:>6}  {:>10.4} {:>+9.4} {:>6}",
            one.centroid,
            one.centroid - exact,
            one.lane,
            two.centroid,
            two.centroid - exact,
            two.lane
        );
    }
    println!();
}

/// One row of phase 2: the sampled lane's ink, at one sample count, swept through a whole
/// pixel of position at a width the lattice does not divide.
///
/// Returns the worst the sampled column reached and the **ladder** — the distinct ink
/// values it produced — which is the answer to question 2 in the form the question asks
/// for.
pub(crate) fn grid_sweep(
    lanes: (&mut Device, &mut Device),
    samples: u32,
    steps: u32,
    worst: &mut Worst,
) -> Vec<f64> {
    let (cpu, gpu) = lanes;
    let case = Case {
        horizontal: true,
        through_transform: true,
        repeated: false,
        filled: false,
        uncached: false,
        width: WITNESS_WIDTH,
    };
    let pitch = 1.0 / f64::from(samples).sqrt();
    println!(
        "== a {WITNESS_WIDTH}-pixel rule at {samples} samples: the grid's pitch is 1/√{samples} = {pitch:.4} =="
    );
    println!(
        "{:>8}  {:>8} {:>8} {:>8}  {:>8} {:>8} {:>8} {:>8}",
        "offset", "cpu ink", "row", "row+1", "gpu ink", "row", "row+1", "centroid"
    );
    let mut ladder: Vec<f64> = Vec::new();
    let mut unpainted = 0_usize;
    let mut positions = 0_usize;
    for offset in offsets(steps) {
        let centre = fixture::BASE + offset;
        let (one, two) = (read(cpu, centre, case), read(gpu, centre, case));
        worst.fold(
            two.centroid - f64::from(centre),
            two.ink - f64::from(WITNESS_WIDTH),
        );
        let (_, ca, cb) = row_split(&one.profile);
        let (row, ga, gb) = row_split(&two.profile);
        push_distinct(&mut ladder, two.ink, byte_drift(samples) + BYTE_SLACK);
        let missed = unpainted_lanes(&one.profile, &two.profile);
        unpainted += missed;
        positions += 1;
        println!(
            "{offset:>8.4}  {:>8.4} {ca:>8.4} {cb:>8.4}  {:>8.4} {ga:>8.4} {gb:>8.4} {:>+8.4}   (row {row}, {missed} unpainted)",
            one.ink,
            two.ink,
            two.centroid - f64::from(centre)
        );
    }
    ladder.sort_by(f64::total_cmp);
    println!("distinct sampled ink values: {ladder:.4?}");
    println!(
        "pixels the shape intersects and the sampled lane left unpainted: {unpainted} over \
         {positions} positions (§10.7.4's first sentence)\n"
    );
    ladder
}

/// Adds `value` to `ladder` unless a value within `apart` of it is there already.
///
/// `apart` is [`byte_drift`] and a byte: two positions that put the same count of samples
/// inside the band can still differ in the last places, and a ladder that counted those
/// separately would have as many rungs as the sweep has steps. It stays well below half a
/// pitch at every sample count this instrument uses, which is what keeps two genuinely
/// different rungs from being merged into one.
fn push_distinct(ladder: &mut Vec<f64>, value: f64, apart: f64) {
    if !ladder.iter().any(|held| (held - value).abs() < apart) {
        ladder.push(value);
    }
}

/// Phase 3: the caller's graph paper — six rules, six commands, at their pitch — measured
/// per rule on both lanes.
pub(crate) fn graph_paper_row(lanes: (&mut Device, &mut Device), width: f32, horizontal: bool) {
    let (cpu, gpu) = lanes;
    let case = Case {
        horizontal,
        through_transform: true,
        repeated: false,
        filled: false,
        uncached: false,
        width,
    };
    println!(
        "== {GRAPH_RULES} {} rules, one command each, pitch {GRAPH_PITCH:.4}, device width {width:.4} ==",
        if horizontal { "horizontal" } else { "vertical" }
    );
    let readings: Vec<Reading> = [&mut *cpu, &mut *gpu]
        .into_iter()
        .map(|target| {
            let scene = graph_paper(target, case, GRAPH_FIRST, GRAPH_PITCH, GRAPH_RULES);
            read_scene(target, &scene, case, Window::Whole)
        })
        .collect();
    println!(
        "{:>5}  {:>10}  {:>10} {:>9}  {:>10} {:>9}",
        "rule", "geometry", "cpu lane", "error", "gpu lane", "error"
    );
    let mut centres = [Vec::new(), Vec::new()];
    for index in 0..GRAPH_RULES {
        let exact = f64::from(GRAPH_PITCH.mul_add(index as f32, GRAPH_FIRST));
        let mut row = [0.0_f64; 2];
        for (column, reading) in readings.iter().enumerate() {
            let (centroid, _) = one_rule(&reading.profile, exact, f64::from(GRAPH_PITCH) / 2.0);
            row[column] = centroid;
            centres[column].push(centroid);
        }
        println!(
            "{index:>5}  {exact:>10.4}  {:>10.4} {:>+9.4}  {:>10.4} {:>+9.4}",
            row[0],
            row[0] - exact,
            row[1],
            row[1] - exact
        );
    }
    for (column, name) in ["cpu", "gpu"].iter().enumerate() {
        let first = centres[column][0];
        let last = centres[column][GRAPH_RULES as usize - 1];
        let pitch = (last - first) / f64::from(GRAPH_RULES - 1);
        println!(
            "{name} lane: measured pitch {pitch:.4} against the document's {GRAPH_PITCH:.4}, \
             lane {}",
            readings[column].lane
        );
    }
    println!();
}
