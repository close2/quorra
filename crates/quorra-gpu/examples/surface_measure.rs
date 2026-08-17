//! The two surface-tier numbers only a person at the real GPU can take.
//!
//! Everything this tree measures for itself is headless, and the brief's success
//! criterion is not: §6.2 defines success as *a third of the CPU backend's 5.9 ms on a
//! dense text page at window resolution, presenting to a surface* — a number that has
//! never been taken, because this account cannot open a window on the owner's display.
//! This example takes it, plus the other owner-only number `HANDOVER.md` names: what a
//! presenting host's **first** frame pays in first-use pipeline compiles, because the
//! warm set is compiled for `Rgba8Unorm` and a surface negotiates `Bgra8Unorm`.
//!
//! # How to run (on the real GPU, at the real display)
//!
//! ```text
//! MESA_SHADER_CACHE_DISABLE=true cargo run --release -p quorra-gpu --example surface_measure
//! ```
//!
//! The cache variable matters for the first-frame half: RADV serves pipeline compiles
//! from `~/.cache/mesa_shader_cache` on every run but the first, and a compile served
//! from disk is exactly the cost a viewer's first launch does not get to skip. Run it
//! once more *without* the variable if you want the warm-cache number too — both are
//! real, they answer different questions, and the header line says which one a run is.
//!
//! # What to paste back
//!
//! The whole output. The lines that decide things:
//!
//! - `first frame … compiles:` — every `pipeline compile (first use)` entry a first
//!   frame absorbed, by name and duration, per round. If these are present on the
//!   surface path after `wait_until_warm`, the warm set is missing the surface's
//!   format and the one-line fix in `spawn_warm_up` has its measurement.
//! - `steady …` — minima and medians over the counted frames. `wall − acquire` is the
//!   §6.2 comparison figure: the surface is configured `PresentMode::Fifo`, so the
//!   acquire blocks on vsync and a raw wall clock measures the display's refresh
//!   rate, not the renderer. Both are printed; neither is subtracted silently.
//! - `steady …, instrumented encode` — the same shape rendered with
//!   `Options::instrument_encode`, so the encode splits into geometry / staging /
//!   recording (ADR 0023). These rounds answer *where the encode goes*; §6.2's figure
//!   comes from the uninstrumented rounds, because the instrument costs a clock read
//!   per seam and a measurement that moves what it measures is not an instrument.
//!
//! # Scenes
//!
//! Two of `quorra-pages`' corpus-measured archetypes at the brief's
//! 1191×1684 — the same definitions `tests/archetypes.rs` gates, not copies of them
//! (ADR 0060): **dense text** (4 320 commands over 818 outlines — §6.2's page shape at
//! the corpus's p99) and **artwork** (900 commands, 8 groups, 4 of them blended — the
//! shape that needs the compositor, so its first frame is the one that compiles
//! `Composite` and `Blit` for the surface's format). Rounds alternate shapes, each on
//! a fresh device, warmed before its first frame — the caller's hand-over order.
//!
//! Traps this binary handles because they have each cost a round before (HANDOVER.md):
//! the load average is printed beside the results (wall clocks lie under load), rounds
//! are round-robin so machine drift falls on both shapes, and steady-state statistics
//! are minima and medians, never means.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use quorra_gpu::{Device, Options, Target, Viewport, WarmUp};
use quorra_pages::{ARTWORK, Archetype, DENSE_TEXT};
use quorra_scene::{Affine, OutlineId, Scene};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

/// The brief's window scale (§6.2): the size both baselines were measured at.
const WIDTH: u32 = 1191;
const HEIGHT: u32 = 1684;

/// The two shapes this instrument alternates, from `quorra-pages` (ADR 0060).
///
/// **§6.2's page** — dense text at the corpus's p99 — and **the grouped shape**, whose
/// first frame is the one that needs `Composite` and `Blit` for the surface's format.
/// Both were private copies of the archetype generator in this file until 2026-08-17;
/// the numbers `doc/PLAN.md` carries for the real-display row were taken on those copies,
/// and the copies were re-cut with the fixture on the same day.
const SHAPES: [&Archetype; 2] = [&DENSE_TEXT, &ARTWORK];

/// Build a shape's scene on this device.
fn build(device: &mut Device, shape: &Archetype) -> Scene {
    let outlines: Vec<OutlineId> = quorra_pages::outlines(shape)
        .iter()
        .map(|path| device.upload_outline(path).expect("an archetype outline"))
        .collect();
    quorra_pages::scene(shape, &outlines, None).expect("an archetype builds")
}

/// A frame's counters as the row `quorra-pages` records, field by named field.
fn recorded(counters: &quorra_gpu::Counters) -> quorra_pages::Recorded {
    quorra_pages::Recorded {
        commands: u64::from(counters.commands),
        commands_culled: u64::from(counters.commands_culled),
        distinct_outlines: u64::from(counters.distinct_outlines),
        atlas_distinct_keys: u64::from(counters.atlas_distinct_keys),
        clip_distinct_regions: u64::from(counters.clip_distinct_regions),
        tiles: u64::from(counters.tiles),
        layer_textures: u64::from(counters.layer_textures),
        clip_residue_regions: u64::from(counters.clip_residue_regions),
        clip_residue_tiles: u64::from(counters.clip_residue_tiles),
        coverage_texels: counters.coverage.texels,
    }
}

// ---------------------------------------------------------------------------
// Measurement records
// ---------------------------------------------------------------------------

/// What one frame reported, host clock and device clock side by side.
struct FrameRecord {
    /// Wall clock around `Device::render` — includes the Fifo acquire block.
    wall: Duration,
    encode: Duration,
    upload: Duration,
    execute: Duration,
    acquire: Duration,
    present: Duration,
    /// The `encode: geometry / staging / recording` split, present on instrumented
    /// rounds and zero otherwise.
    geometry: Duration,
    staging: Duration,
    recording: Duration,
    /// The `pipeline compile (first use)` entries this frame absorbed.
    compiles: Vec<(&'static str, Duration)>,
}

impl FrameRecord {
    fn of(wall: Duration, timings: &quorra_gpu::Timings) -> Self {
        let phase = |name: &str| {
            timings
                .phases
                .iter()
                .find(|(n, _)| *n == name)
                .map_or(Duration::ZERO, |(_, d)| *d)
        };
        Self {
            wall,
            encode: timings.encode,
            upload: timings.upload,
            execute: timings.execute,
            acquire: phase("target acquire"),
            present: phase("present"),
            geometry: phase("encode: geometry"),
            staging: phase("encode: staging"),
            recording: phase("encode: recording"),
            compiles: timings
                .phases
                .iter()
                .filter(|(n, _)| *n == "pipeline compile (first use)")
                .copied()
                .collect(),
        }
    }

    /// The §6.2 comparison figure: the frame's cost with the vsync block removed.
    fn cost(&self) -> Duration {
        self.wall.saturating_sub(self.acquire)
    }

    /// What the wall clock saw and no phase claims: the device wait plus submit
    /// overhead. Large values here mean the frame waited on the GPU or the driver.
    fn unattributed(&self) -> Duration {
        self.wall
            .saturating_sub(self.encode)
            .saturating_sub(self.upload)
            .saturating_sub(self.acquire)
            .saturating_sub(self.present)
    }
}

/// One round: a fresh, warmed device; a first frame; then counted steady frames.
struct Round {
    shape: &'static str,
    instrumented: bool,
    warm_wait: Duration,
    first: FrameRecord,
    first_uploaded: u64,
    steady: Vec<FrameRecord>,
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// Minimum and median of a series, as milliseconds.
fn min_median(series: &mut [f64]) -> (f64, f64) {
    series.sort_by(f64::total_cmp);
    let min = series.first().copied().unwrap_or(0.0);
    let median = series.get(series.len() / 2).copied().unwrap_or(0.0);
    (min, median)
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

struct Config {
    rounds: u32,
    frames: u32,
    adapter: Option<String>,
    /// `--check`: the smallest run that reaches every assertion, and no report.
    ///
    /// Two rounds of two frames each is one frame of each shape's signature gate — the
    /// only assertion this file makes — and nothing else. `cargo test` neither builds
    /// nor runs an example (ADR 0060), and this one is the owner's instrument on the
    /// real display, so what CI can check is that its gates hold, not what it measures.
    /// Forty frames of Fifo against a display's refresh is a measurement and belongs to
    /// the person taking it.
    check: bool,
}

fn config() -> Config {
    let mut config = Config {
        // Two rounds of each of the four configurations (shape × instrumented).
        rounds: 8,
        frames: 40,
        adapter: std::env::var("QUORRA_MEASURE_ADAPTER").ok(),
        check: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        if flag == "--check" {
            config.check = true;
            config.rounds = 2;
            config.frames = 2;
            continue;
        }
        let value = args.next();
        let parsed = value.as_deref().and_then(|v| v.parse::<u32>().ok());
        match (flag.as_str(), parsed) {
            ("--rounds", Some(n)) => config.rounds = n.max(2),
            ("--frames", Some(n)) => config.frames = n.max(1),
            ("--adapter", None | Some(_)) => config.adapter = value,
            _ => {
                eprintln!(
                    "usage: surface_measure [--check] [--rounds N] [--frames N] [--adapter NAME]"
                );
                std::process::exit(2);
            }
        }
    }
    config
}

struct Measure {
    config: Config,
    window: Option<Arc<Window>>,
    device: Option<Device>,
    scene: Option<Scene>,
    round: u32,
    frame_in_round: u32,
    current: Option<Round>,
    finished: Vec<Round>,
}

impl Measure {
    /// Which shape and whether encode is instrumented, round-robin over all four
    /// configurations so machine drift falls on each equally. The uninstrumented
    /// rounds carry §6.2's number (the instrument costs a clock read per seam); the
    /// instrumented ones say where a steady encode goes.
    fn config_for(round: u32) -> (&'static Archetype, bool) {
        let shape = SHAPES[(round % 2) as usize];
        (shape, (round / 2).is_multiple_of(2))
    }

    /// A fresh device for this round, warmed before its first frame — the caller's
    /// hand-over order: the CPU backend draws until `is_warm`, then the GPU takes over.
    fn fresh_device(&mut self, window: &Arc<Window>) -> Device {
        let (shape, plain) = Self::config_for(self.round);
        let options = Options {
            adapter: self.config.adapter.clone(),
            instrument_encode: !plain,
            ..Options::default()
        };
        let device = Device::for_surface(Arc::clone(window), &options).expect("surface device");
        let waited = Instant::now();
        device.wait_until_warm();
        let warm_wait = waited.elapsed();
        match device.warm_up() {
            WarmUp::Refused(problem) => {
                eprintln!("warm-up refused: {problem}");
                std::process::exit(1);
            }
            outcome => eprintln!(
                "round {}: {}{} — warm after {:.2} ms wait ({outcome:?})",
                self.round,
                shape.name,
                if plain { "" } else { ", instrumented" },
                ms(warm_wait),
            ),
        }
        self.current = Some(Round {
            shape: shape.name,
            instrumented: !plain,
            warm_wait,
            first: FrameRecord {
                wall: Duration::ZERO,
                encode: Duration::ZERO,
                upload: Duration::ZERO,
                execute: Duration::ZERO,
                acquire: Duration::ZERO,
                present: Duration::ZERO,
                geometry: Duration::ZERO,
                staging: Duration::ZERO,
                recording: Duration::ZERO,
                compiles: Vec::new(),
            },
            first_uploaded: 0,
            steady: Vec::new(),
        });
        device
    }

    fn render_one(&mut self, window: &Arc<Window>) {
        if self.device.is_none() {
            let device = self.fresh_device(&Arc::clone(window));
            self.device = Some(device);
            let (shape, _) = Self::config_for(self.round);
            let scene = build(self.device.as_mut().expect("just set"), shape);
            self.scene = Some(scene);
        }
        let (device, scene) = (
            self.device.as_mut().expect("set above"),
            self.scene.as_ref().expect("set above"),
        );
        let viewport = Viewport::full(WIDTH, HEIGHT, Affine::IDENTITY);
        let started = Instant::now();
        match device.render(scene, &viewport, Target::Surface) {
            Ok(frame) => {
                let record = FrameRecord::of(started.elapsed(), frame.timings());
                let current = self.current.as_mut().expect("round started");
                if self.frame_in_round == 0 {
                    // The signature gate this instrument never had. The viewport is
                    // `WIDTH × HEIGHT` whatever the window negotiated, so the counters
                    // are the same exact functions of the scene that
                    // `tests/archetypes.rs` compares — and the row is that crate's, so
                    // it cannot go stale here while staying right there (ADR 0060).
                    let (shape, _) = Self::config_for(self.round);
                    assert_eq!(
                        recorded(&frame.counters()),
                        shape.recorded.expect("both shapes are priced pages"),
                        "{}: this is not the page `doc/PLAN.md`'s real-display row is \
                         attributed to",
                        shape.name,
                    );
                    current.first = record;
                    current.first_uploaded = frame.counters().bytes_uploaded;
                } else {
                    current.steady.push(record);
                }
            }
            Err(error) => {
                eprintln!("frame refused: {error}");
                std::process::exit(1);
            }
        }
        self.frame_in_round += 1;
        if self.frame_in_round > self.config.frames {
            self.finished.extend(self.current.take());
            // Drop the device (and its surface) before the next round creates one.
            self.device = None;
            self.scene = None;
            self.frame_in_round = 0;
            self.round += 1;
        }
    }

    fn report(&self) {
        println!("\n== first frames (after wait_until_warm; every compile entry listed) ==");
        for round in &self.finished {
            let compiled: f64 = round.first.compiles.iter().map(|(_, d)| ms(*d)).sum();
            println!(
                "{:>10}{}: wall {:>7.2} ms  (encode {:.2}, upload {:.2}, unattributed {:.2}, \
                 execute {:.2}, acquire {:.2}, present {:.2})  uploaded {:.2} MB  warm wait \
                 {:.2} ms  compiles: {}",
                round.shape,
                if round.instrumented { " (instr)" } else { "" },
                ms(round.first.wall),
                ms(round.first.encode),
                ms(round.first.upload),
                ms(round.first.unattributed()),
                ms(round.first.execute),
                ms(round.first.acquire),
                ms(round.first.present),
                round.first_uploaded as f64 / 1e6,
                ms(round.warm_wait),
                if round.first.compiles.is_empty() {
                    "none".to_owned()
                } else {
                    format!("{} totalling {compiled:.2} ms", round.first.compiles.len())
                },
            );
        }
        for (shape, instrumented) in [
            (DENSE_TEXT.name, false),
            (ARTWORK.name, false),
            (DENSE_TEXT.name, true),
            (ARTWORK.name, true),
        ] {
            let rounds: Vec<&Round> = self
                .finished
                .iter()
                .filter(|r| r.shape == shape && r.instrumented == instrumented)
                .collect();
            if rounds.is_empty() {
                continue;
            }
            let series = |f: &dyn Fn(&FrameRecord) -> Duration| {
                let mut all: Vec<f64> = rounds
                    .iter()
                    .flat_map(|r| r.steady.iter().map(|record| ms(f(record))))
                    .collect();
                min_median(&mut all)
            };
            let (wall_min, wall_med) = series(&|r| r.wall);
            let (cost_min, cost_med) = series(&|r| r.cost());
            let (encode_min, encode_med) = series(&|r| r.encode);
            let (upload_min, upload_med) = series(&|r| r.upload);
            let (idle_min, idle_med) = series(&|r| r.unattributed());
            let (execute_min, execute_med) = series(&|r| r.execute);
            let (acquire_min, acquire_med) = series(&|r| r.acquire);
            let (present_min, present_med) = series(&|r| r.present);
            println!(
                "\n== steady, {shape}{} ({} frames over {} rounds; min / median, ms) ==",
                if instrumented {
                    ", instrumented encode"
                } else {
                    ""
                },
                rounds.iter().map(|r| r.steady.len()).sum::<usize>(),
                rounds.len(),
            );
            println!(
                "  wall             {wall_min:>7.3} / {wall_med:>7.3}   (includes the Fifo acquire block)"
            );
            if instrumented {
                // The instrument costs a clock read per seam; §6.2's figure is the
                // uninstrumented block's, and this block says where the encode goes.
                let (geometry_min, geometry_med) = series(&|r| r.geometry);
                let (staging_min, staging_med) = series(&|r| r.staging);
                let (recording_min, recording_med) = series(&|r| r.recording);
                println!("  encode           {encode_min:>7.3} / {encode_med:>7.3}");
                println!("    geometry       {geometry_min:>7.3} / {geometry_med:>7.3}");
                println!("    staging        {staging_min:>7.3} / {staging_med:>7.3}");
                println!("    recording      {recording_min:>7.3} / {recording_med:>7.3}");
            } else {
                println!(
                    "  wall − acquire   {cost_min:>7.3} / {cost_med:>7.3}   <- §6.2's comparison figure"
                );
                println!("  encode           {encode_min:>7.3} / {encode_med:>7.3}");
            }
            println!("  upload           {upload_min:>7.3} / {upload_med:>7.3}");
            println!(
                "  unattributed     {idle_min:>7.3} / {idle_med:>7.3}   (device wait + submit)"
            );
            println!("  execute          {execute_min:>7.3} / {execute_med:>7.3}");
            println!("  acquire          {acquire_min:>7.3} / {acquire_med:>7.3}");
            println!("  present          {present_min:>7.3} / {present_med:>7.3}");
        }
        println!(
            "\n§6.2's bar, for reading the uninstrumented dense-text block: the CPU backend \
             draws that page in 5.9 ms; a third (2.0 ms) is success, a tenth (0.6 ms) a clear \
             win."
        );
    }
}

impl ApplicationHandler for Measure {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("quorra surface measure")
                        .with_inner_size(winit::dpi::PhysicalSize::new(WIDTH, HEIGHT))
                        .with_resizable(false),
                )
                .expect("window creation on this display"),
        );
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested => {
                let Some(window) = self.window.as_ref().map(Arc::clone) else {
                    return;
                };
                if self.round >= self.config.rounds {
                    event_loop.exit();
                    return;
                }
                self.render_one(&window);
                if self.round >= self.config.rounds {
                    event_loop.exit();
                } else {
                    window.request_redraw();
                }
            }
            WindowEvent::CloseRequested => event_loop.exit(),
            _ => {}
        }
    }
}

fn main() {
    let config = config();
    let load = std::fs::read_to_string("/proc/loadavg")
        .map_or_else(|_| "unavailable".to_owned(), |s| s.trim().to_owned());
    let cache = if std::env::var_os("MESA_SHADER_CACHE_DISABLE").is_some() {
        "disabled (cold-compile numbers: what a first launch pays)"
    } else {
        "ENABLED — compile numbers may be the disk cache's, not the compiler's; \
         re-run with MESA_SHADER_CACHE_DISABLE=true for the first-launch cost"
    };
    println!("surface_measure at {WIDTH}x{HEIGHT}, PresentMode::Fifo (the library's only mode)");
    println!("load average: {load}");
    println!("mesa shader cache: {cache}");

    let event_loop = EventLoop::new().expect("an event loop needs a display; set DISPLAY");
    let mut measure = Measure {
        config,
        window: None,
        device: None,
        scene: None,
        round: 0,
        frame_in_round: 0,
        current: None,
        finished: Vec::new(),
    };
    event_loop.run_app(&mut measure).expect("event loop");

    // Release any leftover device (and its surface) before the summary prints.
    measure.device = None;
    if measure.config.check {
        println!("check: both shapes presented and their counters are the recorded rows");
        return;
    }
    measure.report();
}
