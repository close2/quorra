//! **The two numbers a display that states its own refresh is needed for.**
//!
//! ADR 0056 closes with *"we cannot answer whether it holds the rate"* and ADR 0058 with
//! *"what the pass costs inside a real frame is still not ours to say"*; both were parked
//! because `Xvfb` reports a refresh of 0.00 and 120 Hz cannot be observed under it at
//! all. This module is what asks a real display, and it asks two different kinds of
//! question — which is why it is a separate file from [`crate::arrangement`], where every
//! number is exact arithmetic and none of them is a clock.
//!
//! # The instrument, and why the refresh is measured rather than assumed
//!
//! The library configures the surface `PresentMode::Fifo` and offers no other mode, so
//! **presents are refresh-locked by construction**: `acquire` blocks until the
//! presentation engine releases an image, and one image is released per refresh. Two
//! consequences shape everything below.
//!
//! - The interval between consecutive presents is `max(refresh, what one present costs)`.
//!   So a run of presents with nothing to draw *measures the refresh* — it is the
//!   display's own clock read through the only mode this library has — and every other
//!   number here is quoted against that rather than against a constant someone typed.
//! - While that interval stays at the refresh, the honest question is not "how fast is
//!   the present" but **"does the presenting thread make its window"**, which is a
//!   property and not a duration. It is reported as a count: how many refreshes each
//!   present consumed, quantised against the measured refresh.
//!
//! When the pass is *loaded* past a refresh the interval stops being floored and starts
//! measuring the work, which is what the replication sweep uses: `n` copies of the
//! caller's arrangement cost `n` times the pass, and the `n` at which presents stop
//! landing every refresh brackets what one arrangement costs — a **count**, against the
//! display's own clock, with no host stopwatch deciding anything.
//!
//! # What is asserted and what is only printed
//!
//! ADR 0052's seam is the whole design of this file. The **assertions** are counts:
//! [`crate::arrangement`]'s totals against ADR 0058, that every present succeeded and
//! reported the layer count it was given, and that presents got through a render at all.
//! The **statistics** — intervals, refreshes per present, the sweep — are printed and
//! gated on nothing, because a wall clock on this machine is not a gate (`HANDOVER.md`:
//! a 24.7 → 10.3 ms improvement that re-measured as 19.9 → 20.0 an hour later).

use std::collections::BTreeMap;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use quorra_gpu::{Device, Layer, RenderError, Target, Viewport};
use quorra_pages::DENSE_TEXT;
use quorra_scene::{Affine, Color, ImageFilter, Point, Rect, Scene, SceneBuilder};

use crate::arrangement::{self, Shape};
use crate::{Display, fixture};

/// The window this measures at: the caller's own 2048 × 2560 divided by their device
/// scale of 1.6, because **2560 rows do not fit an 1800-row display**. The counts are
/// recomputed from whatever size the window system actually gives, so this is a request
/// rather than an assumption.
const WINDOW: (u32, u32) = (1280, 1600);

/// Presents used to read the display's refresh off an empty slice.
const PROBE_PRESENTS: usize = 120;

/// How many copies of the caller's arrangement one present draws, per sweep row.
///
/// A geometric sweep because what is wanted is the **crossing** — the row at which the
/// interval leaves the refresh floor — and a crossing between 16 and 32 says as much as
/// one between 16 and 17 while costing a sixteenth of the presents.
const SWEEP: [usize; 7] = [1, 2, 4, 8, 16, 32, 64];

/// Presents per sweep row, per round.
const SWEEP_PRESENTS: usize = 32;

/// Rounds of the whole sweep, round-robin so that machine drift falls on every row
/// rather than on whichever ran while something else started (`HANDOVER.md`).
const SWEEP_ROUNDS: usize = 3;

/// How long the render thread holds the device for, in the cadence phase.
///
/// A stopping rule rather than a measurement: the caller's own frame of that page is
/// 4 454.9 ms and their floor with the whole of `encode` deleted is 107.0 ms, so a span
/// of this order is a short one of theirs. What is reported is the **count** of presents
/// and the count of renders that fitted in it.
const CADENCE_SPAN: Duration = Duration::from_millis(300);

/// What one run of presents observed.
struct Run {
    /// The interval between consecutive presents, in the order they happened. One
    /// shorter than the number of presents, and the first present is not in it — there is
    /// nothing before it to be an interval from.
    intervals: Vec<Duration>,
    /// Presents the swapchain refused and that were retried. Zero on a healthy run, and
    /// printed rather than asserted because a compositor may legitimately outdate a
    /// surface at any moment.
    retried: usize,
    /// Presents that reconfigured the swapchain. Exactly one is expected — the first,
    /// after the resize — and more than that means the window is still settling.
    reconfigured: usize,
}

impl Run {
    /// The middle interval. A median rather than a mean because one scheduling stall
    /// moves a mean and cannot move a median (`HANDOVER.md`).
    fn median(&self) -> Duration {
        let mut sorted = self.intervals.clone();
        sorted.sort_unstable();
        sorted
            .get(sorted.len() / 2)
            .copied()
            .unwrap_or(Duration::ZERO)
    }

    /// The fastest interval observed, which under `Fifo` cannot be shorter than a
    /// refresh and is therefore the tightest estimate of one.
    fn minimum(&self) -> Duration {
        self.intervals
            .iter()
            .copied()
            .min()
            .unwrap_or(Duration::ZERO)
    }

    /// How many refreshes each present consumed, counted. **This is the property the
    /// split is judged on**: an entry of `1` is a present that made its window, and
    /// anything above it is a refresh the window did not get a picture on.
    ///
    /// Everything at or above [`LONG`] is one bucket. A display that states no refresh at
    /// all — `Xvfb` — otherwise produces a bucket per present and a line nobody can read,
    /// and the distinction between 27 refreshes and 28 was never the question.
    fn refreshes(&self, refresh: Duration) -> BTreeMap<u64, usize> {
        let mut counts = BTreeMap::new();
        for interval in &self.intervals {
            let ratio = interval.as_secs_f64() / refresh.as_secs_f64();
            // Rounded rather than truncated: a present that lands 2 % early is a present
            // on its refresh, and `Fifo` puts one there routinely because the interval is
            // measured on the host between two calls and the refresh is the display's.
            let quantised = (ratio.round().max(1.0) as u64).min(LONG);
            *counts.entry(quantised).or_insert(0) += 1;
        }
        counts
    }

    /// The counts above, as a line.
    fn refreshes_line(&self, refresh: Duration) -> String {
        let counts = self.refreshes(refresh);
        let total = self.intervals.len().max(1);
        counts
            .iter()
            .map(|(refreshes, presents)| {
                let name = if *refreshes >= LONG {
                    format!("{LONG}+")
                } else {
                    refreshes.to_string()
                };
                format!(
                    "{name} refresh x{presents} ({:.1} %)",
                    100.0 * *presents as f64 / total as f64
                )
            })
            .collect::<Vec<_>>()
            .join("  ")
    }
}

/// Where the "how many refreshes did this present consume" histogram stops counting.
const LONG: u64 = 4;

/// Presents whose intervals are discarded at the start of every run.
///
/// **A swapchain has more than one image**, so the first few presents do not block: they
/// fill the queue and return immediately, and an interval measured across them is a
/// statement about the queue's depth rather than about the display. `Fifo` reaches its
/// steady state — one present released per refresh — once every image is in flight, which
/// is at most `desired_maximum_frame_latency + 1` presents away.
const SETTLE_PRESENTS: usize = 4;

/// Present `layers` `count` times after the swapchain has settled, timing each present's
/// return.
///
/// The event loop is pumped once per present because that is what a host does and a
/// window nobody pumps is a window the server stops believing in — its cost is on the
/// presenting thread and therefore inside every interval reported, which is the honest
/// place for it.
fn present_run(
    display: &mut Display,
    presenter: &mut quorra_gpu::Presenter,
    layers: &[Layer<'_>],
    count: usize,
) -> Run {
    let mut intervals = Vec::with_capacity(count);
    let mut retried = 0;
    let mut reconfigured = 0;
    let mut previous: Option<Instant> = None;
    let mut done = 0;
    while done < count + SETTLE_PRESENTS {
        display.pump();
        match presenter.present(layers) {
            Ok(()) => {}
            // A surface can be outdated by the window system between two presents; that
            // is a retry by name and not a failure (`RenderError::SurfaceUnavailable`).
            Err(RenderError::SurfaceUnavailable { .. }) => {
                retried += 1;
                continue;
            }
            Err(other) => panic!("presenting {} layers: {other:?}", layers.len()),
        }
        let now = Instant::now();
        let cost = presenter.last().expect("a present just succeeded");
        assert_eq!(
            cost.layers,
            layers.len(),
            "a present must report the slice it was given"
        );
        reconfigured += usize::from(cost.reconfigured);
        if let Some(previous) = previous {
            intervals.push(now - previous);
        }
        previous = Some(now);
        done += 1;
    }
    Run {
        intervals,
        retried,
        reconfigured,
    }
}

/// A texture for one shape of the arrangement, filled once so that nothing here samples
/// a texture whose contents were never written.
fn layer_texture(device: &mut Device, shape: Shape, tint: Color) -> wgpu::Texture {
    let texture = fixture::layer_texture(device, shape.extent, shape.name);
    let mut builder = SceneBuilder::new();
    builder
        .rect(
            Rect::new(
                Point::new(0.0, 0.0),
                Point::new(shape.extent.0 as f32, shape.extent.1 as f32),
            ),
            Affine::IDENTITY,
            tint,
            None,
            None,
        )
        .expect("a rectangle the size of its own texture");
    device
        .render(
            &builder.finish(),
            &Viewport::full(shape.extent.0, shape.extent.1, Affine::IDENTITY),
            Target::Texture(&texture),
        )
        .expect("a layer's own contents");
    texture
}

/// The layers of an arrangement, in the order they are drawn.
fn layers_of<'a>(shapes: &[Shape], textures: &'a [wgpu::Texture]) -> Vec<Layer<'a>> {
    shapes
        .iter()
        .zip(textures)
        .map(|(shape, texture)| Layer {
            texture,
            placement: Affine::translate(shape.at.0, shape.at.1),
            filter: ImageFilter::Nearest,
        })
        .collect()
}

/// `copies` copies of one arrangement, which is how the sweep loads the pass: the same
/// slice repeated draws the same fragments again, so `n` copies cost `n` passes and the
/// arithmetic between rows is a multiplication rather than a model.
fn replicated<'a>(layers: &[Layer<'a>], copies: usize) -> Vec<Layer<'a>> {
    let mut all = Vec::with_capacity(layers.len() * copies);
    for _ in 0..copies {
        all.extend(layers.iter().copied());
    }
    all
}

/// Everything this module measures, in the order it measures it.
///
/// Takes the device and gives it back, because the cadence phase sends it to another
/// thread — which is the arrangement under test rather than an implementation detail.
pub(crate) fn measure(
    display: &mut Display,
    device: Device,
    presenter: &mut quorra_gpu::Presenter,
    check: bool,
) -> Device {
    arrangement::the_shapes_are_the_ones_adr_0058_counted();

    let window = display.resize(WINDOW);
    presenter.resize(window.0, window.1);
    let content = arrangement::scaled_to(window);
    let today = arrangement::window_sized(&content, window);

    let mut device = device;
    let tints = [
        fixture::FIELD,
        fixture::MARK,
        fixture::CHROME,
        fixture::FIELD,
    ];
    let textures: Vec<wgpu::Texture> = today
        .iter()
        .zip(tints)
        .map(|(shape, tint)| layer_texture(&mut device, *shape, tint))
        .collect();
    let layers = layers_of(&today, &textures);

    let refresh = probe(display, presenter, check, window, &content, &today);
    let device = cadence(display, device, presenter, &layers, refresh, check);
    sweep(display, presenter, &layers, refresh, check);
    device
}

/// The display's own refresh, read through the only present mode this library has.
fn probe(
    display: &mut Display,
    presenter: &mut quorra_gpu::Presenter,
    check: bool,
    window: (u32, u32),
    content: &[Shape],
    today: &[Shape],
) -> Duration {
    let count = if check { 6 } else { PROBE_PRESENTS };
    let run = present_run(display, presenter, &[], count);
    // The **median** rather than the minimum. Under `Fifo` neither can be shorter than a
    // refresh, so the minimum looks like the tighter estimate — but a display that paces
    // nothing at all has no floor for it to find, and one interval that got through
    // between two compositor frames would then divide every other number here. The median
    // of a run of presents with nothing to draw is the rate the display released images
    // at, which is the quantity wanted. Both are printed.
    let refresh = run.median();
    assert!(
        refresh > Duration::ZERO,
        "an empty present must still cost a refresh under Fifo; the display reported none"
    );
    if check {
        return refresh;
    }
    println!("\n-- ADR 0056 and ADR 0058, at a display that states its own refresh --");
    println!(
        "window {} x {} (the caller's 2048 x 2560 at scale {} does not fit 2880 x 1800)",
        window.0,
        window.1,
        arrangement::THEIR_SCALE
    );
    println!(
        "refresh, measured over {count} empty presents: median {:.3} ms ({:.2} Hz), \
         min {:.3} ms",
        refresh.as_secs_f64() * 1e3,
        1.0 / refresh.as_secs_f64(),
        run.minimum().as_secs_f64() * 1e3,
    );
    println!("the pass, in fragments (exact — arithmetic over the placements):");
    println!(
        "{}",
        arrangement::row("window-sized overlays (today)", today, window)
    );
    println!(
        "{}",
        arrangement::row("content-sized overlays", content, window)
    );
    refresh
}

/// **ADR 0056's number**: how many presents land while a render holds the device, and
/// whether each of them made its refresh.
fn cadence(
    display: &mut Display,
    device: Device,
    presenter: &mut quorra_gpu::Presenter,
    layers: &[Layer<'_>],
    refresh: Duration,
    check: bool,
) -> Device {
    let span = if check { Duration::ZERO } else { CADENCE_SPAN };
    let (started, has_started) = mpsc::channel();
    let (finished, has_finished) = mpsc::channel();
    let render = thread::spawn(move || {
        let mut device = device;
        let scene = dense_text(&mut device);
        let target = fixture::layer_texture(&device, (DENSE_TEXT.width, DENSE_TEXT.height), "page");
        let viewport = Viewport::full(DENSE_TEXT.width, DENSE_TEXT.height, Affine::IDENTITY);
        started.send(()).expect("the main thread is waiting");
        let began = Instant::now();
        let mut renders = 0_u32;
        loop {
            device
                .render(&scene, &viewport, Target::Texture(&target))
                .expect("dense text renders while the window is presented");
            renders += 1;
            if began.elapsed() >= span {
                break;
            }
        }
        finished
            .send((renders, began.elapsed()))
            .expect("the main thread is waiting");
        device
    });

    has_started.recv().expect("the render thread started");
    let began = Instant::now();
    let mut intervals = Vec::new();
    let mut previous = began;
    let mut presents = 0_u32;
    let (renders, held) = loop {
        display.pump();
        if presenter.present(layers).is_ok() {
            let now = Instant::now();
            intervals.push(now - previous);
            previous = now;
            presents += 1;
        }
        if let Ok(done) = has_finished.try_recv() {
            break done;
        }
    };
    let device = render.join().expect("the render thread finished");
    assert!(
        presents >= 1,
        "the point of the split is presenting during a render; none got through"
    );
    if check {
        return device;
    }
    let run = Run {
        intervals,
        retried: 0,
        reconfigured: 0,
    };
    println!(
        "\npresents through {renders} renders of dense text holding the device for {:.1} ms:",
        held.as_secs_f64() * 1e3
    );
    println!(
        "  {presents} presents, {:.2} per render, {:.2} per refresh of the span",
        f64::from(presents) / f64::from(renders),
        f64::from(presents) / (held.as_secs_f64() / refresh.as_secs_f64()),
    );
    println!(
        "  refreshes each present consumed: {}",
        run.refreshes_line(refresh)
    );
    println!(
        "  interval min {:.3} ms, median {:.3} ms",
        run.minimum().as_secs_f64() * 1e3,
        run.median().as_secs_f64() * 1e3,
    );
    device
}

/// **ADR 0058's number**: what share of a refresh the present pass takes, bracketed by
/// the number of copies of the caller's arrangement that still land every refresh.
fn sweep(
    display: &mut Display,
    presenter: &mut quorra_gpu::Presenter,
    layers: &[Layer<'_>],
    refresh: Duration,
    check: bool,
) {
    let rows: &[usize] = if check { &[1] } else { &SWEEP };
    let presents = if check { 6 } else { SWEEP_PRESENTS };
    let rounds = if check { 1 } else { SWEEP_ROUNDS };
    let mut best: BTreeMap<usize, Run> = BTreeMap::new();
    for _ in 0..rounds {
        for copies in rows {
            let slice = replicated(layers, *copies);
            // **The instrument's own gate**, and the only assertion in this file that can
            // fail for a reason the display does not decide: the whole sweep reads a
            // crossing off `n` times the work, so a replication that quietly produced
            // fewer layers would move the crossing and look exactly like a faster pass.
            assert_eq!(
                slice.len(),
                layers.len() * copies,
                "n = {copies} must draw {} layers",
                layers.len() * copies
            );
            let run = present_run(display, presenter, &slice, presents);
            match best.get(copies) {
                Some(previous) if previous.median() <= run.median() => {}
                _ => {
                    best.insert(*copies, run);
                }
            }
        }
    }
    if check {
        return;
    }
    println!(
        "\nthe present pass, loaded: n copies of the caller's four layers in one present\n\
         (minima of {rounds} round-robin rounds of {presents} presents; a row still at 1.00 \
         refreshes is a pass that fits one)"
    );
    println!("      n   median interval   refreshes/present   per copy   retried  reconfig");
    for (copies, run) in &best {
        let median = run.median().as_secs_f64() * 1e3;
        let ratio = run.median().as_secs_f64() / refresh.as_secs_f64();
        // A row still on the floor says nothing about the pass: its interval is the
        // display's, so dividing it by `n` measures the refresh divided by `n`. Naming
        // that is the whole point of printing the column at all.
        let per_copy = if ratio < FLOORED {
            "  (floored)".to_owned()
        } else {
            format!("{:>7.3} ms", median / *copies as f64)
        };
        println!(
            "  {copies:>5}   {median:>13.3} ms   {ratio:>17.2}   {per_copy}   {:>7}  {:>8}",
            run.retried, run.reconfigured,
        );
    }
    let held = best
        .iter()
        .filter(|(_, run)| run.median().as_secs_f64() / refresh.as_secs_f64() < FLOORED)
        .map(|(copies, _)| *copies)
        .max();
    match held {
        Some(copies) => println!(
            "  the largest n whose presents still land every refresh is {copies}, \
             so one present of the caller's four layers is at most 1/{copies} of a refresh \
             ({:.3} ms of {:.3})",
            refresh.as_secs_f64() * 1e3 / copies as f64,
            refresh.as_secs_f64() * 1e3,
        ),
        None => println!("  no row held the refresh; the pass exceeds one at a single copy"),
    }
    print_the_slope(&best, refresh);
}

/// Where a row stops being floored by the display and starts measuring the pass.
///
/// A row that lands every refresh reads 1.00 and a row that misses one reads at least
/// 1.33 in every run taken so far, so the cut is placed between them rather than at the
/// arithmetic 2.0 a "missed a whole refresh" reading would suggest: `Fifo`'s steady state
/// when the work exceeds a refresh is the **work**, not the next multiple of the refresh —
/// images become available at the rate they are produced and the queue simply drains.
const FLOORED: f64 = 1.1;

/// What one copy costs, read as the **slope** between the two most loaded rows.
///
/// Every row below the crossing is floored at the refresh and says nothing about the
/// pass — `Fifo` is what makes it a floor and the floor is the point. Above it the
/// interval is the work, and the difference between two loaded rows divides out the
/// per-present cost that does not scale with the layer count: the acquire, the clear, the
/// submission and the event pump. It is a difference of two host clocks and it is
/// **indicative**, which is why the count above is stated first.
fn print_the_slope(best: &BTreeMap<usize, Run>, refresh: Duration) {
    let loaded: Vec<(usize, Duration)> = best
        .iter()
        .filter(|(_, run)| run.median().as_secs_f64() / refresh.as_secs_f64() >= FLOORED)
        .map(|(copies, run)| (*copies, run.median()))
        .collect();
    let [.., (low, at_low), (high, at_high)] = loaded.as_slice() else {
        println!("  fewer than two rows left the refresh floor; there is no slope to read");
        return;
    };
    let per_copy = (at_high.as_secs_f64() - at_low.as_secs_f64()) / (high - low) as f64;
    println!(
        "  slope between n={low} and n={high}: {:.3} ms per copy, {:.1} % of a refresh \
         (indicative — two host clocks)",
        per_copy * 1e3,
        100.0 * per_copy / refresh.as_secs_f64(),
    );
}

/// §6.2's page — dense text at the corpus's p99 — built on this device.
///
/// Built on the render thread rather than handed to it: `upload_outline` needs the
/// `&mut Device` that the thread now owns, and the whole point of the arrangement is that
/// the main thread does not have one.
fn dense_text(device: &mut Device) -> Scene {
    let outlines: Vec<quorra_scene::OutlineId> = quorra_pages::outlines(&DENSE_TEXT)
        .iter()
        .map(|path| device.upload_outline(path).expect("an archetype outline"))
        .collect();
    quorra_pages::scene(&DENSE_TEXT, &outlines, None).expect("an archetype builds")
}
