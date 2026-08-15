//! What a §7.10.5 program's generated shader costs to compile, by the program's length
//! (ADR 0053).
//!
//! `doc/spike-function-paint.md` §3 is the only figure this feature has ever had — 6.3 ms
//! for the seven-segment witness on RADV with a cold driver cache — and it was measured
//! against the *spike's* emitter, not against `src/function/generate.rs`. This binary is
//! that number taken here.
//!
//! ```text
//! cargo run --release -p quorra-gpu --example function_compile [-- <adapter substring> [rounds]]
//! ```
//!
//! # What is timed, and why it is a span rather than a subtraction
//!
//! `PipelineStore::function_pipeline` brackets exactly the three steps a cache miss pays —
//! generate the WGSL, parse it, build the pipeline — and the frame that paid names the
//! result in its own `Timings::phases` as `"function shader compile (first use)"`. So this
//! reads a **direct span** of the work, which is what `doc/HANDOVER.md` asks for before any
//! wall clock is believed. `pipeline.rs`'s `captured` blocks on the validation scope inside
//! that span, so the driver's own compile is inside it too rather than deferred past the
//! measurement.
//!
//! # Three things this gets wrong if they are not deliberate
//!
//! - **The driver's on-disk shader cache keys on the compiled SPIR-V** — the spike lost a
//!   round to it (§3's methodological note). Every sample here is therefore a program no
//!   process has ever compiled: the literals are seeded from the process's start instant,
//!   so a second run of this binary cannot read the first run's cache. The shape is
//!   identical between runs and only the constants move, which is what makes the samples
//!   comparable while keeping every one of them a cold compile.
//! - **The first pipeline of a process pays what the driver defers until then.** The spike
//!   measured 64.5 ms against 6.3 for exactly that reason. Round 0 is therefore reported
//!   apart and never enters a minimum.
//! - **Minima of round-robin rounds, never means**, with the load average printed either
//!   side of every sample. This machine is somebody's desktop; a reader who does not like
//!   the load discounts the run rather than the conclusion. Note what the printed value can
//!   and cannot say: the kernel recomputes `/proc/loadavg` every five seconds and a default
//!   run of this binary finishes inside one of those ticks, so every sample of a run
//!   usually prints one number. It dates the *run*, and the round-robin is what keeps a
//!   drift inside the run from landing on one length rather than on all four.
//!
//! # What a length is, here
//!
//! The spike's witness is 482 instructions with 23 branches and a maximum stack depth of 8
//! (§1's table). A program of that *length* is built below at the same branch density,
//! because a branch is what a generated shader spends a block on and a straight-line
//! program of the same length would be an easier shader than any real one. It is still not
//! the witness — that is a PDF stream in the caller's tree and its compiled form is theirs
//! to produce — so what this measures is **our emitter's output at a stated length and
//! branch density**, and the comparison with the spike's row is a comparison of two
//! programs of one length.

// A measurement binary's arithmetic is instruction counts and indices, all far below the
// bound `MAX_PROGRAM_LENGTH` states; the library's own lints are stricter because its
// inputs come from a document.
#![allow(
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects
)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use quorra_gpu::{Device, Frame, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Compose, FillRule, FnOp, FnRange, FunctionId, OutlineId, Paint, Point, Rect,
    Scene, SceneBuilder, Segment,
};

/// The target is small on purpose: the quantity under test is a compile, and every
/// fragment this binary shades is time spent proving nothing.
const SIZE: u32 = 64;

/// `pi_seven_segment.pdf`, the longer of the caller's two witnesses:
/// `doc/spike-function-paint.md` §1's table gives 482 instructions and 23 branches. Both
/// numbers are used — the first as a length to measure at, the second as the density that
/// makes a program of that length shaped like a real one.
const WITNESS_LENGTH: usize = 482;
const WITNESS_BRANCHES: usize = 23;

/// The lengths measured, in the order printed: the floor, a quarter of the witness, the
/// witness's own length, and twice it — enough to say whether the cost is linear in the
/// program without claiming a model the samples cannot support.
const LENGTHS: [usize; 4] = [1, 121, WITNESS_LENGTH, 2 * WITNESS_LENGTH];

/// A program of exactly `length` instructions that leaves three values for an `Rgb` range,
/// at the witness's branch density, with `seed` fixing every literal it contains.
///
/// # The shape, and the stack invariant that makes it admissible
///
/// The two values a type 1 shading pushes are on the stack when the program starts, and
/// every unit below leaves the stack exactly as it found it — `[x, y']`, both reals — so
/// the two paths of a branch agree on their operand types and `analyse` can decide them
/// (`function/typing.rs`). One trailing `PushReal` makes the three §7.10.1 components
/// `Analysis::admits` requires, which since `doc/notes-function-wiring.md` §2.1 is an
/// equality rather than a floor.
///
/// - a **straight** unit is `PushReal(c) add`, two instructions, and the sum accumulates
///   onto a fragment input so no compiler can fold the chain away;
/// - a **branch** unit is `dup PushReal(c) gt {PushReal(c) add} if`, six instructions,
///   lowering to a real block in the generated WGSL;
/// - one `abs` pads the parity when `length` is not otherwise reachable, since both units
///   are of even length and the tail is odd.
///
/// Every operator is exact, so the program's `Agreement` is `Bounded` and `upload_function`
/// admits it (ADR 0053 §3) — the classification is not what is being measured here.
fn program_of(length: usize, seed: f32) -> Vec<FnOp> {
    // A literal that varies with the position it is emitted at and with the seed, kept in
    // a narrow band around a half so no constant is degenerate enough for a driver to
    // treat specially.
    let literal = |at: usize| 0.25 + seed + (at % 17) as f32 * 0.01;

    let branches = (length * WITNESS_BRANCHES).div_euclid(WITNESS_LENGTH);
    let branches = branches.min(length.saturating_sub(1) / 6);
    let rest = length - 1 - branches * 6;
    let (parity, straights) = (rest % 2, rest / 2);

    let mut program = Vec::with_capacity(length);
    for _ in 0..branches {
        let at = program.len();
        program.extend([
            FnOp::Dup,
            FnOp::PushReal(literal(at)),
            FnOp::Gt,
            // The instruction after the two-instruction body: forward, and inside the
            // program, which is `check_program`'s condition on every jump.
            FnOp::JumpUnless {
                target: (at + 6) as u32,
            },
            FnOp::PushReal(literal(at + 4)),
            FnOp::Add,
        ]);
    }
    for _ in 0..straights {
        let at = program.len();
        program.extend([FnOp::PushReal(literal(at)), FnOp::Add]);
    }
    if parity == 1 {
        program.push(FnOp::Abs);
    }
    program.push(FnOp::PushReal(literal(program.len())));
    program
}

fn outline(device: &mut Device) -> OutlineId {
    let side = SIZE as f32;
    device
        .upload_outline(&[
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(side, 0.0)),
            Segment::LineTo(Point::new(side, side)),
            Segment::LineTo(Point::new(0.0, side)),
            Segment::Close,
        ])
        .expect("a rectangle is a valid outline")
}

fn page(outline: OutlineId, program: FunctionId) -> Scene {
    let mut builder = SceneBuilder::new();
    builder
        .fill(
            outline,
            Affine::IDENTITY,
            FillRule::NonZero,
            Paint::Function {
                program,
                domain: Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
                matrix: Affine::scale(SIZE as f32, SIZE as f32),
                range: FnRange::Rgb([[0.0, 1.0]; 3]),
                background: None,
            },
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )
        .expect("a valid function fill");
    builder.finish()
}

/// The measurement target, created once: a frame that allocated one would be measuring the
/// allocator alongside the compile.
fn target_texture(device: &Device) -> wgpu::Texture {
    let (gpu, _) = device.wgpu();
    gpu.create_texture(&wgpu::TextureDescriptor {
        label: Some("function compile measurement target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// The one phase this binary exists to read, summed over the frame.
///
/// A sum rather than a first: a frame that compiled twice would otherwise report half of
/// what it paid, and reading a number that quietly drops a sample is how a measurement
/// round gets published wrong. One placement of one program under one style compiles once,
/// so the sum is a single compile and the count below says so.
fn compiled(frame: &Frame) -> (Duration, usize) {
    let named = frame
        .timings()
        .phases
        .iter()
        .filter(|(name, _)| *name == "function shader compile (first use)");
    named.fold((Duration::ZERO, 0), |(total, count), (_, each)| {
        (total + *each, count + 1)
    })
}

fn load_average() -> String {
    std::fs::read_to_string("/proc/loadavg").map_or_else(
        |_| "unknown".into(),
        |line| {
            line.split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ")
        },
    )
}

/// One sample: a program no process has compiled, drawn once, then released so that the
/// pipelines its hash keyed are dropped (`PipelineStore::forget_program`) rather than
/// accumulating over the run.
fn sample(
    device: &mut Device,
    outline: OutlineId,
    texture: &wgpu::Texture,
    length: usize,
    seed: f32,
) -> Duration {
    let program = device
        .upload_function(&program_of(length, seed))
        .expect("the generated program is admitted");
    let scene = page(outline, program);
    let viewport = Viewport::full(SIZE, SIZE, Affine::IDENTITY);

    let before = load_average();
    let frame = device
        .render(&scene, &viewport, Target::Texture(texture))
        .expect("the function page draws");
    let (elapsed, count) = compiled(&frame);
    let after = load_average();
    drop(frame);
    device.release(program).expect("the program is resident");

    println!(
        "  {length:>4} instructions  {elapsed:>12?}  {count} compile(s)  \
         load {before} before / {after} after"
    );
    elapsed
}

fn main() {
    let mut args = std::env::args().skip(1);
    let adapter = args.next();
    let rounds: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(12);

    let mut device = Device::headless(&Options {
        adapter,
        ..Options::default()
    })
    .expect("an adapter");
    // The fixed table's own compiles are not this measurement, and a background warm-up
    // thread still running would be contending with it.
    device.wait_until_warm();
    println!("adapter: {}", device.description());

    let outline = outline(&mut device);
    let texture = target_texture(&device);
    // Distinct SPIR-V for every sample of every run: the driver's on-disk cache is what
    // turned a 1 500 ms compile into a 1.1 ms one for the spike, and it keys on the
    // compiled form rather than on the WGSL text. The clock is used for nothing else.
    let run = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos() as usize);
    let seed_of =
        |round: usize, at: usize| ((run + round * LENGTHS.len() + at) % 4096) as f32 / 8192.0;

    let mut minima: Vec<Option<Duration>> = vec![None; LENGTHS.len()];
    for round in 0..=rounds {
        let apart = if round == 0 {
            " — reported apart: the first pipeline of a process pays what the driver deferred"
        } else {
            ""
        };
        println!("round {round}{apart}");
        for (at, length) in LENGTHS.iter().enumerate() {
            let elapsed = sample(&mut device, outline, &texture, *length, seed_of(round, at));
            if round > 0 {
                let best = &mut minima[at];
                *best = Some(best.map_or(elapsed, |best| best.min(elapsed)));
            }
        }
    }

    println!("\nminima over {rounds} round-robin rounds, first round excluded:");
    for (length, best) in LENGTHS.iter().zip(&minima) {
        match best {
            Some(best) => println!("  {length:>4} instructions  {best:?}"),
            None => println!("  {length:>4} instructions  no sample"),
        }
    }
    println!("load average at the end: {}", load_average());
}
