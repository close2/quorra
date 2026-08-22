//! **Where each coverage lane puts a hairline, against where the geometry says.**
//!
//! The caller's `QUORRA_FEEDBACK.md` §31 reports that our two coverage lanes disagree
//! about the *placement* of an axis-aligned rule about one device pixel wide, by up to an
//! eighth of a device pixel, on four of their corpus pages — with the amount of ink right
//! on both and only the position wrong. They ask two questions, and this is the instrument
//! that answers them from our own arithmetic rather than from their pages:
//!
//! 1. is the default (`Coverage::Cpu`) lane's per-command offset intended, and if it is a
//!    quantisation, what is its bound?
//! 2. is the `Coverage::Gpu` lane's y coverage quantised, and to what?
//!
//! # What it draws, and why a stroke rather than a rectangle
//!
//! One horizontal rule, `WIDTH` device pixels thick, swept through a whole pixel of
//! sub-pixel position in `STEPS` steps; then the same in the other axis. A **stroke**,
//! because `quorra_scene::axis_aligned_rect` recognises a rectangle's four edges and a
//! solid fill of one takes the analytic rectangle lane, which is exact by construction and
//! is not the lane the question is about (`tests/scale_invariance.rs` states the same
//! reason for the same choice).
//!
//! # What it measures
//!
//! For each position, per lane: the **centroid** of the rule's coverage along the axis it
//! is swept in, and the **total ink** in units of a fully covered pixel. Both are compared
//! against the exact values, which for a band of known width and position are arithmetic
//! and not another rasterisation — `expected_centroid` is the band's own midpoint and
//! `expected_ink` is its area. That is what makes this a check against the definition
//! (ADR 0005's area rule) rather than a comparison of two implementations.
//!
//! ```text
//! cargo run --release -p quorra-gpu --example lane_placement
//! ```
//!
//! `--check` runs the smallest sweep that still exercises both axes and both lanes.

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

use quorra_gpu::{Coverage, DEFAULT_GLYPH_QUANTUM, Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, LineCap, LineJoin, Paint, Point, Scene, SceneBuilder, Segment, Stroke,
};

/// What a byte of alpha is worth as a centroid error on this fixture: a coverage rounded
/// to 1/255 moves the measured centre by about this much, and it is not the quantum's
/// doing. Measured at 0.0019 across the whole sweep with the quantum's own error at zero.
const BYTE_SLACK: f64 = 0.004;

/// An atlas eight of a hairline's tiles cannot fit, in bytes — see [`device`].
const TINY_ATLAS: u64 = 1024;

/// The target, in device pixels. Small on purpose: the rule spans it, so every row of the
/// raster carries the same statement and the centroid is read over the whole picture.
const SIDE: u32 = 128;

/// The rule's device width, in pixels — the caller's population is "about one device pixel
/// wide", which is also where a sampled grid has the least to work with.
const WIDTH: f32 = 1.0;

/// Sub-pixel positions swept across one whole pixel.
///
/// **A prime, and that is the point.** The atlas quantises a phase to
/// `Options::glyph_quantum` of a pixel (default 1/16), so a sweep of 16 steps lands every
/// sample exactly on the quantisation grid and measures zero error at every one of them —
/// which is what the first run of this instrument did. A step that shares no factor with
/// the quantum visits the whole of each bucket instead.
const STEPS: u32 = 37;

/// The caller's own witness CTM: `bug1743245.pdf`'s graph paper is drawn under a uniform
/// scale of this, one `q … cm … S … Q` per rule (their §31.2).
const CTM: f32 = 0.317_180_62;

/// Where the rule sits, before the sub-pixel offset is added: far enough from the edges
/// that neither cap nor target boundary is in the measurement.
const BASE: f32 = 80.0;

/// Lanes below this are not read: it is where a decoy placement is put and the measured
/// rule is not (see [`profile`]).
const WINDOW: u32 = 60;

/// How far a decoy placement sits from the measured rule, in device pixels — clear of
/// [`WINDOW`] on the other side, so neither mark can reach the other's half.
const DECOY_GAP: f32 = 40.0;

/// A device on the named coverage setting, optionally with an atlas too small to hold a
/// rule's tile.
///
/// **The small atlas is what reaches the sampled lane at all**, and it took a reading of
/// `take_gpu_lane` to find it: that predicate declines the device lane whenever the tile is
/// `worth_caching`, so a mark the atlas would take is drawn by the processor under *either*
/// setting. `CacheProspect::TooLarge` is the one answer that is never worth caching, so an
/// atlas smaller than eight of these tiles is the only way a hairline this size is ever
/// asked of the grid. That is a fact about the lane chooser worth knowing before reading
/// any comparison of the two settings: **on the default atlas they are the same lane.**
fn device(coverage: Coverage, atlas: Option<u64>) -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        coverage,
        atlas_budget: atlas.unwrap_or(Options::default().atlas_budget),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this runs")
}

/// A rule across the target at `centre`, in the axis named by `horizontal`.
///
/// Each call is its own scene *and its own command*, which is the unit the caller's
/// finding is stated in: "the offset is constant within one drawing command and different
/// between commands".
///
/// `through_transform` is the difference between the two ways a host can state the same
/// picture, and it is the whole reason this instrument has a flag: the position can be in
/// the outline's own coordinates, or the outline can sit at the origin and the *command's*
/// affine can carry it — which is what a PDF's `q … cm … S … Q` produces and therefore what
/// the caller's four pages are made of.
fn rule(device: &mut Device, centre: f32, case: Case) -> Scene {
    let Case {
        horizontal,
        through_transform,
        repeated,
        filled,
        ..
    } = case;
    // The caller's witness is `0.5-unit strokes under a 0.317 CTM`, so when the position
    // is stated through the affine it is stated the way their page states it: the outline
    // in the user space that CTM scales, not in device pixels.
    let stated = if through_transform { 0.0 } else { centre };
    let span = (f32::from(SIDE as u16) + 4.0) / if through_transform { CTM } else { 1.0 };
    let (from, to) = if horizontal {
        (Point::new(-span, stated), Point::new(span, stated))
    } else {
        (Point::new(stated, -span), Point::new(stated, span))
    };
    let placement = if !through_transform {
        Affine::IDENTITY
    } else if horizontal {
        Affine::scale(CTM, CTM).then(Affine::translate(0.0, centre))
    } else {
        Affine::scale(CTM, CTM).then(Affine::translate(centre, 0.0))
    };
    if filled {
        return band(device, horizontal, placement, span);
    }
    let outline = device
        .upload_outline(&[Segment::MoveTo(from), Segment::LineTo(to)])
        .unwrap();
    let mut builder = SceneBuilder::new();
    let stroke = Stroke {
        width: WIDTH,
        cap: LineCap::Butt,
        join: LineJoin::Miter,
        miter_limit: 10.0,
    };
    let ink = Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0));
    builder
        .stroke(
            outline,
            placement,
            stroke,
            ink,
            None,
            BlendMode::Normal,
            None,
        )
        .unwrap();
    // **The second placement is the whole of what `repeated` means.** `CacheProspect`
    // answers `worth_caching` on a key used *once* in a frame with `false` (ADR 0065), so a
    // fixture drawing one rule can never reach the atlas — and the atlas is where a phase
    // is quantised. Graph paper is one outline drawn many times, so a decoy placement of
    // the same outline is what makes this fixture the caller's page rather than a hairline
    // on its own. It is drawn far from the measured rule, in the half of the target no
    // profile reads.
    if repeated {
        let decoy = if horizontal {
            Affine::translate(0.0, -DECOY_GAP)
        } else {
            Affine::translate(-DECOY_GAP, 0.0)
        };
        builder
            .stroke(
                outline,
                placement.then(decoy),
                stroke,
                ink,
                None,
                BlendMode::Normal,
                None,
            )
            .unwrap();
    }
    builder.finish()
}

/// The same rule as a **fill** of its own band, which is the other way a host can state a
/// hairline and the only one that can reach the sampled lane.
///
/// Six points rather than four, and the extra two are collinear: `axis_aligned_rect`
/// accepts `M,L,L,L[,L-back-to-start][,Close]` and refuses anything longer
/// (`geom/segment.rs`), so a sixth point keeps the geometry an exact axis-aligned band
/// while sending it to the coverage lane instead of the analytic rectangle one. The band
/// is the shape; the recogniser is what is being avoided, not the definition.
fn band(device: &mut Device, horizontal: bool, placement: Affine, span: f32) -> Scene {
    // In the outline's own space, which the placement scales — the band sits at zero and
    // the affine carries it, exactly as the stroke above does. Getting this wrong is how an
    // earlier run of this instrument reported a six-pixel "error" that was a user-space
    // half-width added to a device-space centre.
    let half = WIDTH / 2.0 / CTM;
    let (lo, hi) = (-half, half);
    let point = |along: f32, across: f32| {
        if horizontal {
            Point::new(along, across)
        } else {
            Point::new(across, along)
        }
    };
    let outline = device
        .upload_outline(&[
            Segment::MoveTo(point(-span, lo)),
            Segment::LineTo(point(0.0, lo)),
            Segment::LineTo(point(span, lo)),
            Segment::LineTo(point(span, hi)),
            Segment::LineTo(point(0.0, hi)),
            Segment::LineTo(point(-span, hi)),
            Segment::Close,
        ])
        .unwrap();
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            placement,
            quorra_scene::FillRule::NonZero,
            Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0)),
            None,
            BlendMode::Normal,
            quorra_scene::Compose::SrcOver,
            None,
        )
        .unwrap();
    builder.finish()
}

/// The coverage of each row (or column) of the raster, in units of a fully covered pixel,
/// summed across the other axis.
fn profile(pixels: &[u8], horizontal: bool) -> Vec<f64> {
    let mut lanes = vec![0.0_f64; SIDE as usize];
    for y in 0..SIDE {
        for x in 0..SIDE {
            let alpha = pixels[((y * SIDE + x) * 4 + 3) as usize];
            let index = if horizontal { y } else { x } as usize;
            lanes[index] += f64::from(alpha) / 255.0;
        }
    }
    // Per pixel of the rule's length, so the numbers are coverages and not areas.
    for lane in &mut lanes {
        *lane /= f64::from(SIDE);
    }
    // **The window, and why it is in the profile rather than in the sum.** A decoy
    // placement (see `rule`) is a second rule this measurement must not see, and cutting
    // it out of the *sum* cuts the measured rule too when the two are near a boundary —
    // which is what an earlier version of this instrument did, reporting a half-pixel
    // "error" that was its own window truncating the mark. The decoy sits below `WINDOW`
    // and the measured rule well above it, so discarding whole lanes is exact.
    for lane in lanes.iter_mut().take(WINDOW as usize) {
        *lane = 0.0;
    }
    lanes
}

/// The centroid of a coverage profile, and its total — the two numbers the caller's §31
/// tables are stated in.
fn centroid_and_ink(profile: &[f64]) -> (f64, f64) {
    let ink: f64 = profile.iter().sum();
    if ink == 0.0 {
        return (f64::NAN, 0.0);
    }
    let weighted: f64 = profile
        .iter()
        .enumerate()
        // The pixel's own centre: a pixel wholly covered contributes its middle, which is
        // what makes a band from 16.0 to 17.0 read as 16.5 rather than as 16.
        .map(|(index, coverage)| (index as f64 + 0.5) * coverage)
        .sum();
    (weighted / ink, ink)
}

/// One row of the sweep: how the picture is stated, and what the atlas is allowed to do
/// with it. A struct rather than four `bool` parameters, because every one of them changes
/// which lane answers and a reader of a call site should see which is which.
#[derive(Clone, Copy)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "each field is an independent axis of the sweep and every one of them changes \
              which lane answers; named fields at the call site are the point, and are what \
              this struct replaced four positional `bool` parameters with"
)]
struct Case {
    /// A rule across the target, rather than down it.
    horizontal: bool,
    /// The position is carried by the command's affine (a PDF's `cm`) rather than by the
    /// outline's own coordinates.
    through_transform: bool,
    /// A second placement of the same outline, so its key is not single-use.
    repeated: bool,
    /// A fill of the rule's own band rather than a stroke of its centre line.
    filled: bool,
    /// An atlas too small to hold the tile, which is what makes the sampled lane reachable.
    uncached: bool,
}

/// One lane's answer for one placement.
struct Reading {
    centroid: f64,
    ink: f64,
    /// Which of §1.1's lanes made the coverage, so that a number is never attributed to a
    /// lane the mark did not take.
    lane: &'static str,
}

fn read(device: &mut Device, centre: f32, case: Case) -> Reading {
    let scene = rule(device, centre, case);
    let frame = device
        .render(
            &scene,
            &Viewport::full(SIDE, SIDE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("a hairline is within every budget");
    let lanes = frame.counters().lanes;
    let lane = match (lanes.rectangle, lanes.glyph, lanes.path) {
        (0, 0, _) => "path",
        (0, _, 0) => "glyph",
        (_, 0, 0) => "rect",
        _ => "mixed",
    };
    let pixels = frame
        .into_raster()
        .expect("a Readback frame carries a raster")
        .into_pixels();
    let (centroid, ink) = centroid_and_ink(&profile(&pixels, case.horizontal));
    Reading {
        centroid,
        ink,
        lane,
    }
}

/// The four rows of the sweep, in the order that isolates one thing at a time: a stroke
/// (the path lane), the same rule as a fill (which the atlas takes), the same fill with the
/// atlas refusing it (the only way to the sampled lane), and that again in the other axis.
const CASES: [Case; 4] = [
    Case {
        horizontal: true,
        through_transform: true,
        repeated: false,
        filled: false,
        uncached: false,
    },
    Case {
        horizontal: true,
        through_transform: true,
        repeated: false,
        filled: true,
        uncached: false,
    },
    Case {
        horizontal: true,
        through_transform: true,
        repeated: false,
        filled: true,
        uncached: true,
    },
    Case {
        horizontal: false,
        through_transform: true,
        repeated: false,
        filled: true,
        uncached: true,
    },
];

/// How one row of the sweep says what it is.
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
            (true, true, _) => "as a fill the atlas refuses (the sampled lane)",
            (true, false, _) => "as a fill of its own band",
            (false, _, true) => "as a stroke, drawn twice (the atlas can hold it)",
            (false, _, false) => "as a stroke, drawn once (single-use: no atlas)",
        }
    )
}

/// The sub-pixel positions one row visits: a uniform sweep, **plus the top of the last
/// quantum bucket**.
///
/// That extra position is not decoration and it is not negotiable. The only phases the
/// carry of ADR 0073 touches are `fx ≥ 1 − 1/2q` — 3.1 % of the pixel at the default
/// quantum — and a uniform sweep of four steps visits 0, ¼, ½ and ¾, none of which is in
/// it. The assertion in `main` therefore *could not fail for its own regression* under
/// `--check` until this function existed, which is ADR 0052's rule and was checked by
/// forcing the defect and watching `--check` stay green.
fn offsets(steps: u32) -> Vec<f32> {
    let quantum = f32::from(DEFAULT_GLYPH_QUANTUM);
    let mut positions: Vec<f32> = (0..steps)
        .map(|step| f32::from(step as u16) / steps as f32)
        .collect();
    // Inside the top half-bucket, where `(fx · q).round()` reaches `q` itself.
    positions.push(1.0 - 1.0 / (4.0 * quantum));
    positions
}

/// One row: both settings at every sub-pixel position, printed, and the worst error each
/// lane reached folded into `worst` and `worst_ink`.
fn sweep(
    lanes: (&mut Device, &mut Device),
    case: Case,
    steps: u32,
    worst: &mut [f64; 2],
    worst_ink: &mut [f64; 2],
) {
    let (cpu, gpu) = lanes;
    println!("{}", describe(case));
    println!(
        "{:>8}  {:>10}  {:>10} {:>9} {:>6}  {:>10} {:>9} {:>6}",
        "offset", "geometry", "cpu lane", "error", "lane", "gpu lane", "error", "lane"
    );
    for offset in offsets(steps) {
        let centre = BASE + offset;
        let (one, two) = (read(cpu, centre, case), read(gpu, centre, case));
        let exact = f64::from(centre);
        let errors = [one.centroid - exact, two.centroid - exact];
        let inks = [one.ink, two.ink];
        for index in 0..2 {
            if errors[index].abs() > worst[index].abs() {
                worst[index] = errors[index];
            }
            let ink_error = inks[index] - f64::from(WIDTH);
            if ink_error.abs() > worst_ink[index].abs() {
                worst_ink[index] = ink_error;
            }
        }
        println!(
            "{offset:>8.4}  {exact:>10.4}  {:>10.4} {:>+9.4} {:>6}  {:>10.4} {:>+9.4} {:>6}",
            one.centroid, errors[0], one.lane, two.centroid, errors[1], two.lane
        );
    }
    println!();
}

fn main() {
    let check = std::env::args().any(|arg| arg == "--check");
    let steps = if check { 4 } else { STEPS };
    let mut cpu = device(Coverage::Cpu, None);
    let mut gpu = device(Coverage::Gpu, None);
    // Eight of a hairline's tiles do not fit this, so `CacheProspect` answers `TooLarge`
    // and the sampled lane becomes reachable (see `device`).
    let mut cpu_uncached = device(Coverage::Cpu, Some(TINY_ATLAS));
    let mut gpu_uncached = device(Coverage::Gpu, Some(TINY_ATLAS));
    println!("a {WIDTH}-pixel rule swept through one pixel of position, {steps} steps");
    println!("centroid error is (lane − geometry) in device pixels; ink is in covered pixels\n");

    let mut worst = [0.0_f64; 2];
    let mut worst_ink = [0.0_f64; 2];
    for case in CASES {
        let lanes = if case.uncached {
            (&mut cpu_uncached, &mut gpu_uncached)
        } else {
            (&mut cpu, &mut gpu)
        };
        sweep(lanes, case, steps, &mut worst, &mut worst_ink);
    }
    println!(
        "worst centroid error: cpu lane {:+.4}, gpu lane {:+.4}",
        worst[0], worst[1]
    );
    println!(
        "worst ink error:      cpu lane {:+.4}, gpu lane {:+.4}",
        worst_ink[0], worst_ink[1]
    );
    // The bound the atlas's quantum states (ADR 0009, ADR 0073): a placement is rounded to
    // the nearest of `q` buckets, so it moves by at most half of one. Asserted rather than
    // printed, because it is the property the carry defect broke and the reason this
    // example is in CI's `--check` loop.
    let bound = 1.0 / (2.0 * f64::from(DEFAULT_GLYPH_QUANTUM));
    for (index, name) in ["cpu", "gpu"].iter().enumerate() {
        assert!(
            worst[index].abs() <= bound + BYTE_SLACK,
            "the {name} lane moved a hairline by {:+.4} device pixels, past the quantum's \
             own bound of {bound:.4}",
            worst[index]
        );
    }
    println!("both lanes are inside the quantum's bound of {bound:.4} device pixels");
}
