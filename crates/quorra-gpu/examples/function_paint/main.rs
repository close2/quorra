//! A feasibility spike: is a §7.10.5 function a paint the device should evaluate?
//!
//! `/home/cl/projects/pdf-viewer/doc/QUORRA_FUNCTION_PAINT.md` asks for a paint whose
//! colour is a small program, and says its §3 — "a function-based shading is a
//! fragment shader written in another language" — is an intuition rather than a
//! number. This example is the number. It is **not** a library feature and nothing in
//! `crates/quorra-gpu/src` changed to host it; the write-up is
//! `doc/spike-function-paint.md`.
//!
//! It measures the two shapes their §4 leaves to us, on their two witness programs,
//! on both adapters:
//!
//! - **(i) an interpreter** — one shader, a switch over the instruction list, the
//!   program uploaded as a buffer. Nothing compiles on the frame path.
//! - **(ii) a generated shader per distinct program**, whose operand stack is a set
//!   of named `var`s rather than an indexed array. Pays a compile.
//!
//! Run: `cargo run --release -p quorra-gpu --example function_paint`
//!
//! The comparison it exists to serve is §1 of their document: 30.8 ms of device time
//! and 1 142 ms of scene building for one page of one shading, against `mutool draw
//! -r 96` at 15–16 ms for the whole page.

// A spike's arithmetic is page coordinates and instruction counts, all bounded and
// all exact in the types they use; the library's own lints are stricter because its
// inputs are not. `unwrap`/`expect` are absent regardless — a spike that panics
// reports nothing.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects
)]

mod eval;
mod fixture;
mod harness;
mod measure;
mod program;
mod refusal;
mod report;
mod walk;

use std::time::Duration;

use harness::{Canvas, Gpu, Paint};
use measure::Variant;
use program::Program;
use refusal::Refusal;
use walk::{Facts, Mode};

/// One page drawn by one variant, kept for the cross-adapter comparison.
type Drawn = (String, Vec<u8>);

/// Everything one adapter drew at page size.
type AdapterRasters = (String, Vec<Drawn>);

/// Operand-stack slots. Both shapes get the same number so the comparison is fair,
/// and a program needing more is refused by name before the frame — which is the
/// interpreter's one structural limit, since a WGSL array needs a constant size.
const SLOTS: usize = 64;

/// §6.2's page, and the 4× the corpus gate runs at.
const SIZES: [(&str, u32, u32); 2] = [("1191x1684", 1191, 1684), ("4x", 4764, 6736)];

/// Enough rounds to find a minimum; fewer when a round is expensive.
const ROUNDS: usize = 12;

/// How long one size's round-robin may take on one adapter.
const BUDGET: Duration = Duration::from_secs(25);

/// A variant whose *projected* pass at the next size up exceeds this is not run
/// there. Not a nicety: at 4× the interpreter's pass exceeded the driver's reset
/// watchdog and took the whole device with it — "the CS has been cancelled because
/// the context is lost. This context is guilty of a hard recovery."
const WATCHDOG: Duration = Duration::from_millis(900);

const HEAD: &str = include_str!("head.wgsl");
const OPS: &str = include_str!("ops.wgsl");
const INTERP: &str = include_str!("interp.wgsl");
const TAIL: &str = include_str!("tail.wgsl");

/// One witness program, compiled and analysed for both shapes.
struct Case {
    name: &'static str,
    bytes: usize,
    program: Program,
    /// Shape (i)'s view: the resolved list and what it needs.
    interpreted: Facts,
    /// Shape (ii)'s view, or the reason it was refused.
    generated: Result<Facts, Refusal>,
}

fn main() {
    println!("# quorra spike: a device-evaluated §7.10.5 function paint\n");
    report::load();

    let cases = match load_cases() {
        Ok(cases) => cases,
        Err(reason) => {
            println!("no witness to measure: {reason}");
            return;
        }
    };
    report::programs(&cases);
    report::refusals();

    let reference = report::cpu_reference(&cases);

    let mut rasters: Vec<AdapterRasters> = Vec::new();
    for filter in ["RADV", "llvmpipe"] {
        match Gpu::open(filter) {
            Some(gpu) => {
                let drawn = run_adapter(&gpu, &cases, &reference);
                rasters.push((gpu.name.clone(), drawn));
            }
            None => println!("\n## no adapter matching `{filter}`\n"),
        }
    }
    report::cross_adapter(&rasters);
}

/// Read, compile and analyse both witnesses.
fn load_cases() -> Result<Vec<Case>, String> {
    let root = fixture::root();
    let mut cases = Vec::new();
    for witness in &fixture::WITNESSES {
        let source =
            fixture::read(&root, witness).map_err(|why| format!("{}: {why}", witness.name))?;
        // §8.7.4.5.2's type 1 shading: two inputs, and DeviceRGB's three components out.
        let program =
            program::compile(&source, 2, 3).map_err(|why| format!("{}: {why}", witness.name))?;
        let interpreted = walk::walk(&program, SLOTS, Mode::Analyse)
            .map_err(|why| format!("{}: {why}", witness.name))?;
        let generated = walk::walk(&program, SLOTS, Mode::Generate);
        cases.push(Case {
            name: witness.name,
            bytes: witness.length,
            program,
            interpreted,
            generated,
        });
    }
    Ok(cases)
}

fn interpreter_source() -> String {
    let body = INTERP
        .replace("%OPCODES%", &program::wgsl_opcode_constants())
        .replace("%SLOTS%", &SLOTS.to_string());
    format!("{HEAD}\n{OPS}\n{body}\n{TAIL}")
}

fn generated_source(evaluate: &str) -> String {
    format!("{HEAD}\n{OPS}\n{evaluate}\n{TAIL}")
}

/// How many times each shader is compiled, so the compile column has a minimum
/// rather than a sample. `doc/HANDOVER.md`: quote minima, never a single wall clock.
const COMPILE_ROUNDS: usize = 3;

/// Compile every shape this adapter will draw with, round-robin, keeping minima.
///
/// The round-robin matters more here than anywhere else in the spike: a compile is
/// host work on a machine whose load average swings between 2 and 50, and it is the
/// number `PLAN.md` §1.8 and the caller's §5.2 both judge shape (ii) by.
fn compile_all(gpu: &Gpu, cases: &[Case]) -> (Paint, Vec<Option<Paint>>) {
    let mut sources = vec![(
        "shape (i)  interpreter".to_string(),
        interpreter_source(),
        true,
    )];
    for case in cases {
        if let Some(wgsl) = case.generated.as_ref().ok().and_then(|f| f.wgsl.as_deref()) {
            sources.push((
                format!("shape (ii) {}", case.name),
                generated_source(wgsl),
                false,
            ));
        }
    }

    // Which shader a process compiles *first* is a question and not a detail: with a
    // cold driver cache the first pipeline of a process pays what the driver defers
    // until then, and attributing that to whichever shader happened to be first is
    // exactly the mistake `doc/HANDOVER.md` warns about. The order is switchable so
    // the question can be answered rather than argued.
    if std::env::var_os("QUORRA_FUNCTION_PAINT_ORDER").is_some() {
        sources.reverse();
    }

    let mut built: Vec<Option<Paint>> = (0..sources.len()).map(|_| None).collect();
    let mut best = vec![(Duration::MAX, Duration::MAX); sources.len()];
    let mut first = vec![(Duration::ZERO, Duration::ZERO); sources.len()];
    for round in 0..COMPILE_ROUNDS {
        for (index, (_, source, with_program)) in sources.iter().enumerate() {
            // A distinct source per round, differing only in a comment. Without it
            // the second round would measure the driver's on-disk shader cache —
            // which is exactly the mistake this spike made once and caught: the same
            // interpreter compiled in 1 500 ms in one process and 1.1 ms in the next.
            let source = format!("{source}\n// round {round}\n");
            let paint = harness::build(gpu, &source, *with_program);
            if round == 0 {
                first[index] = (paint.module, paint.link);
            }
            best[index].0 = best[index].0.min(paint.module);
            best[index].1 = best[index].1.min(paint.link);
            built[index] = Some(paint);
        }
    }
    println!(
        "{:<26} {:>7} {:>10} {:>10} {:>10} {:>10}",
        "shader", "bytes", "module ms", "first ms", "pipeline", "first ms"
    );
    for (((label, source, _), (module, link)), (first_module, first_link)) in
        sources.iter().zip(&best).zip(&first)
    {
        println!(
            "{label:<26} {:>7} {:>10.2} {:>10.2} {:>10.2} {:>10.2}",
            source.len(),
            module.as_secs_f64() * 1e3,
            first_module.as_secs_f64() * 1e3,
            link.as_secs_f64() * 1e3,
            first_link.as_secs_f64() * 1e3
        );
    }

    if std::env::var_os("QUORRA_FUNCTION_PAINT_ORDER").is_some() {
        built.reverse();
    }
    let mut built = built.into_iter();
    let interpreter = built.next().flatten();
    let mut generated = Vec::new();
    for case in cases {
        let has = case.generated.as_ref().ok().and_then(|f| f.wgsl.as_ref());
        generated.push(if has.is_some() {
            built.next().flatten()
        } else {
            None
        });
    }
    // The interpreter is built unconditionally and `COMPILE_ROUNDS` is not zero, so
    // this is a `Paint`; the fallback keeps the spike reporting rather than panicking.
    let interpreter = match interpreter {
        Some(paint) => paint,
        None => harness::build(gpu, &interpreter_source(), true),
    };
    (interpreter, generated)
}

/// Everything one adapter has to say.
fn run_adapter(gpu: &Gpu, cases: &[Case], reference: &[(Duration, Vec<u8>)]) -> Vec<Drawn> {
    println!("\n## {}\n", gpu.name);
    let (interpreter, generated) = compile_all(gpu, cases);

    let programs: Vec<wgpu::Buffer> = cases
        .iter()
        .map(|case| upload_program(gpu, &case.interpreted.ops))
        .collect();

    let mut projected: Vec<Duration> = Vec::new();
    let mut drawn = Vec::new();
    for (index, (label, width, height)) in SIZES.into_iter().enumerate() {
        let scale = if index == 0 {
            1.0
        } else {
            f64::from(width) * f64::from(height) / (f64::from(SIZES[0].1) * f64::from(SIZES[0].2))
        };
        let built = Built {
            cases,
            interpreter: &interpreter,
            generated: &generated,
            programs: &programs,
        };
        let (times, rasters) = measure_size(
            gpu,
            &built,
            (label, width, height),
            reference,
            &projected,
            scale,
        );
        projected = times;
        if index == 0 {
            drawn = rasters;
        }
    }
    drawn
}

/// One thing to draw: which case it paints, with which shape, through which pipeline.
struct Plan<'a> {
    label: String,
    case: usize,
    paint: &'a Paint,
    program: Option<&'a wgpu::Buffer>,
}

/// Every (case, shape) pair, in a stable order both sizes agree on.
fn plan<'a>(
    cases: &[Case],
    interpreter: &'a Paint,
    generated: &'a [Option<Paint>],
    programs: &'a [wgpu::Buffer],
) -> Vec<Plan<'a>> {
    let mut plan = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        plan.push(Plan {
            label: format!("(i)  interp  {}", case.name),
            case: index,
            paint: interpreter,
            program: Some(&programs[index]),
        });
        if let Some(paint) = &generated[index] {
            plan.push(Plan {
                label: format!("(ii) gen     {}", case.name),
                case: index,
                paint,
                program: None,
            });
        }
    }
    plan
}

/// Everything one adapter has built and can draw with.
struct Built<'a> {
    cases: &'a [Case],
    interpreter: &'a Paint,
    generated: &'a [Option<Paint>],
    programs: &'a [wgpu::Buffer],
}

/// One placement: the round-robin table, and — at page size — the agreement check.
///
/// Returns each plan entry's device time, so the next size up can refuse in advance
/// anything whose projection would trip the driver's reset watchdog.
fn measure_size(
    gpu: &Gpu,
    built: &Built<'_>,
    size: (&str, u32, u32),
    reference: &[(Duration, Vec<u8>)],
    previous: &[Duration],
    scale: f64,
) -> (Vec<Duration>, Vec<Drawn>) {
    let Built {
        cases,
        interpreter,
        generated,
        programs,
    } = *built;
    let (label, width, height) = size;
    let canvas = Canvas::new(gpu, width, height);
    let uniforms: Vec<wgpu::Buffer> = cases
        .iter()
        .map(|case| upload_uniforms(gpu, case, width, height))
        .collect();
    let plan = plan(cases, interpreter, generated, programs);

    let admitted: Vec<bool> = plan
        .iter()
        .enumerate()
        .map(|(index, _)| {
            previous
                .get(index)
                .is_none_or(|before| before.mul_f64(scale) < WATCHDOG)
        })
        .collect();
    let binds: Vec<wgpu::BindGroup> = plan
        .iter()
        .zip(&admitted)
        .filter(|(_, keep)| **keep)
        .map(|(entry, _)| {
            bind(
                gpu,
                &entry.paint.layout,
                &uniforms[entry.case],
                entry.program,
            )
        })
        .collect();
    let variants: Vec<Variant<'_>> = plan
        .iter()
        .zip(&admitted)
        .filter(|(_, keep)| **keep)
        .zip(&binds)
        .map(|((entry, _), bind)| Variant {
            label: entry.label.clone(),
            paint: entry.paint,
            bind,
        })
        .collect();

    let best = measure::round_robin(gpu, &canvas, &variants, ROUNDS, BUDGET);
    println!("\n### {label} ({} px)\n", width * height);
    println!(
        "{:<24} {:>12} {:>12} {:>14}",
        "variant", "device ms", "submit ms", "ns per px"
    );
    for (variant, best) in variants.iter().zip(&best) {
        let device = best.device.unwrap_or(best.wall);
        println!(
            "{:<24} {:>12.3} {:>12.3} {:>14.3}",
            variant.label,
            device.as_secs_f64() * 1e3,
            best.wall.as_secs_f64() * 1e3,
            device.as_nanos() as f64 / f64::from(width * height)
        );
    }
    for (index, (entry, keep)) in plan.iter().zip(&admitted).enumerate() {
        if !keep {
            let projection = previous
                .get(index)
                .copied()
                .unwrap_or_default()
                .mul_f64(scale);
            println!(
                "{:<24} not run: it projects to {:.0} ms here, and a pass that long loses the device",
                entry.label,
                projection.as_secs_f64() * 1e3
            );
        }
    }

    let mut measured = Vec::new();
    let mut at = 0;
    for (index, keep) in admitted.iter().enumerate() {
        if *keep {
            measured.push(best[at].device.unwrap_or(best[at].wall));
            at += 1;
        } else {
            measured.push(previous.get(index).copied().unwrap_or_default());
        }
    }

    let drawn = if label == SIZES[0].0 {
        report_agreement(gpu, &canvas, &variants, &plan, reference)
    } else {
        Vec::new()
    };
    (measured, drawn)
}

/// Draw each variant once more, read it back, and compare with the processor.
///
/// This is `QUORRA_FUNCTION_PAINT.md` §5.1 — "the sharp one" — as a count rather than
/// as a worry: how many device pixels does the device disagree about, and by how much.
fn report_agreement(
    gpu: &Gpu,
    canvas: &Canvas,
    variants: &[Variant<'_>],
    plan: &[Plan<'_>],
    reference: &[(Duration, Vec<u8>)],
) -> Vec<Drawn> {
    println!("\nagreement with the processor's own evaluation of the same list:");
    let mut drawn = Vec::new();
    let admitted: Vec<&Plan<'_>> = plan
        .iter()
        .filter(|entry| variants.iter().any(|v| v.label == entry.label))
        .collect();
    for (index, variant) in variants.iter().enumerate() {
        let _ = measure::round_robin(gpu, canvas, &variants[index..=index], 1, BUDGET);
        let pixels = harness::read_pixels(gpu, canvas);
        let Some((_, expected)) = admitted
            .get(index)
            .and_then(|entry| reference.get(entry.case))
        else {
            continue;
        };
        let found = measure::agreement(&pixels, expected);
        println!(
            "  {:<24} exact {:>9}  off-by-one {:>7}  differing {:>7}  worst {:>3}",
            variant.label, found.exact, found.off_by_one, found.differing, found.worst
        );
        drawn.push((variant.label.clone(), pixels));
    }
    drawn
}

fn bind(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    program: Option<&wgpu::Buffer>,
) -> wgpu::BindGroup {
    let mut entries = vec![wgpu::BindGroupEntry {
        binding: 0,
        resource: uniforms.as_entire_binding(),
    }];
    if let Some(program) = program {
        entries.push(wgpu::BindGroupEntry {
            binding: 1,
            resource: program.as_entire_binding(),
        });
    }
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("function paint"),
        layout,
        entries: &entries,
    })
}

fn upload_program(gpu: &Gpu, ops: &[program::Op]) -> wgpu::Buffer {
    let words: Vec<u32> = Program {
        ops: ops.to_vec(),
        inputs: 2,
        outputs: 3,
    }
    .wire();
    let mut bytes = Vec::with_capacity(words.len() * 4);
    for word in words {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("function paint program"),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    gpu.queue.write_buffer(&buffer, 0, &bytes);
    buffer
}

/// The placement: the whole target is the shading's unit square, so a fragment's own
/// coordinate is its parameter. §8.7.4.5.2's Domain for both witnesses is `[0 1 0 1]`.
fn upload_uniforms(gpu: &Gpu, case: &Case, width: u32, height: u32) -> wgpu::Buffer {
    let mut data = Vec::with_capacity(48);
    let push = |data: &mut Vec<u8>, value: f32| data.extend_from_slice(&value.to_le_bytes());
    push(&mut data, 1.0 / width as f32);
    push(&mut data, 0.0);
    push(&mut data, 0.0);
    push(&mut data, 1.0 / height as f32);
    push(&mut data, 0.0);
    push(&mut data, 0.0);
    data.extend_from_slice(&(case.program.ops.len() as u32).to_le_bytes());
    data.extend_from_slice(&(case.program.outputs as u32).to_le_bytes());
    for edge in [0.0, 1.0, 0.0, 1.0] {
        push(&mut data, edge);
    }
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("function paint uniforms"),
        size: data.len() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    gpu.queue.write_buffer(&buffer, 0, &data);
    buffer
}
