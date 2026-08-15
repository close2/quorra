//! The generated WGSL, compiled and run, against an independent host evaluation of the same
//! program.
//!
//! This is the test that checks the *arithmetic* — `function_lowering.rs` checks the slot
//! allocation with the arithmetic held constant, and neither test alone would find a wrong
//! `ps_round`.
//!
//! # Why a compute pass over a buffer rather than a raster
//!
//! `doc/spike-function-paint.md` §5 measured 246 044 texels off by one between RADV and an
//! independent evaluation, and none of them were the program: they were ADR 0006's 8-bit
//! store conversion, one step, on one adapter. A raster puts that conversion between the
//! shader and the assertion and costs the test all of its resolution. Writing `vec4<f32>` to
//! a storage buffer removes it, so a difference this test sees is a difference the *program*
//! produced.
//!
//! # What a failure here means, and what it does not
//!
//! Bitwise equality is asserted only for programs with no inexact operator in them. For the
//! rest the tolerance is explicit and generous, because WGSL §15.7.4.1 licenses the
//! disagreement: 2.5 ULP on `div`, 4 096 ULP on `atan`, an absolute 2⁻¹¹ on `sin`. A tighter
//! bound would be a promise about a driver rather than a check on this crate.
//!
//! The test is skipped, loudly, where no adapter is available.

#![allow(
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects,
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "byte offsets and dispatch counts over a sample grid whose size this file fixes; \
              every expected value is an exact number a clause states, so an epsilon would \
              weaken the assertion into one three different rounding rules pass; and the \
              helpers are scaffolding whose panic is the test failing"
)]

mod function_support;

use function_support::programs::{self, Witness};
use function_support::reference;
use quorra_gpu::function::{
    Agreement, analyse, background_rgba, domain_bounds, generate, range_bounds,
};
use quorra_gpu::wgpu;
use quorra_scene::Color;

/// The harness that calls the generated function once per point.
///
/// Its only job is to be uninteresting: one invocation per point, no interpolation, no
/// attachment, no format conversion.
const HARNESS: &str = r"
@group(0) @binding(0) var<storage, read> points: array<vec2<f32>>;
@group(0) @binding(1) var<storage, read_write> colours: array<vec4<f32>>;
@group(0) @binding(2) var<uniform> bounds: array<vec4<f32>, 4>;

@compute @workgroup_size(64)
fn evaluate_points(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if (index >= arrayLength(&points)) { return; }
    let point = points[index];
    colours[index] = quorra_function_evaluate(
        point.x, point.y, bounds[0], bounds[1].xyz, bounds[2].xyz, bounds[3]);
}
";

/// A device, or the reason there is not one.
fn device() -> Option<(wgpu::Device, wgpu::Queue, String)> {
    let instance = quorra_gpu::create_instance();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .ok()?;
    let name = adapter.get_info().name;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("quorra function tests"),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .ok()?;
    Some((device, queue, name))
}

/// The four `vec4<f32>` the emitted function's parameters are packed into, in the order the
/// harness above unpacks them: the domain rectangle, the range's low and high bounds, and the
/// background. The padding lanes exist because a uniform array's stride is 16 bytes.
fn bounds_of(witness: &Witness) -> Vec<u8> {
    let (low, high) = range_bounds(witness.range);
    let domain = domain_bounds(witness.domain);
    let background = background_rgba(witness.background);
    let values: [f32; 16] = [
        domain[0],
        domain[1],
        domain[2],
        domain[3],
        low[0],
        low[1],
        low[2],
        0.0,
        high[0],
        high[1],
        high[2],
        0.0,
        background[0],
        background[1],
        background[2],
        background[3],
    ];
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// Run one program on the device at every sample point.
fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    witness: &Witness,
    points: &[(f32, f32)],
) -> Vec<[f32; 4]> {
    let analysis = analyse(&witness.program).expect("the witness should be admitted");
    let shader = generate(&analysis, witness.range).expect("the range should be admitted");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(witness.name),
        source: wgpu::ShaderSource::Wgsl(format!("{}{HARNESS}", shader.module()).into()),
    });

    let point_bytes: Vec<u8> = points
        .iter()
        .flat_map(|(x, y)| [x.to_le_bytes(), y.to_le_bytes()])
        .flatten()
        .collect();
    let bound_bytes = bounds_of(witness);
    let output_bytes = (points.len() * 16) as u64;

    let input = buffer(device, queue, &point_bytes, wgpu::BufferUsages::STORAGE);
    let bounds = buffer(device, queue, &bound_bytes, wgpu::BufferUsages::UNIFORM);
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("function colours"),
        size: output_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("function readback"),
        size: output_bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(witness.name),
        layout: None,
        module: &module,
        entry_point: Some("evaluate_points"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: bounds.as_entire_binding(),
            },
        ],
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(points.len().div_ceil(64) as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output_bytes);
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the device should answer the map");
    let mapped = slice.get_mapped_range().expect("the readback should map");
    let colours = mapped
        .chunks_exact(16)
        .map(|chunk| {
            let component = |index: usize| {
                f32::from_le_bytes(chunk[index * 4..index * 4 + 4].try_into().unwrap())
            };
            [component(0), component(1), component(2), component(3)]
        })
        .collect();
    drop(mapped);
    readback.unmap();
    colours
}

fn buffer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bytes: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes.len().max(16) as u64,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, bytes);
    buffer
}

/// Every witness, on whatever adapter is here, against the host evaluation.
///
/// The two assertions differ by classification, which is the point: `Agreement::Bounded` means
/// no inexact operator's value reaches an amplifier, and for a program with *no* inexact
/// operator at all the two sides should land on the same bits.
#[test]
fn the_device_computes_what_the_host_computes() {
    let Some((device, queue, adapter)) = device() else {
        eprintln!("no adapter available; the device half of the function tests did not run");
        return;
    };
    let points = programs::sample_points();
    let mut checked = 0usize;
    // Counted, not assumed: a test whose strict branch never fires is a test that passed
    // because it asked an easier question, and nothing in the assertions above would say so.
    let mut compared_bitwise = 0usize;

    for witness in programs::all() {
        let analysis = analyse(&witness.program).unwrap();
        let colours = run(&device, &queue, &witness, &points);
        assert_eq!(colours.len(), points.len(), "{}", witness.name);

        let bitwise = analysis.agreement() == Agreement::Bounded
            && !uses_an_inexact_operator(&witness.program);
        if bitwise {
            compared_bitwise += 1;
        }
        for ((x, y), got) in points.iter().copied().zip(&colours) {
            let want = reference::evaluate_shading(&witness, x, y);
            for channel in 0..4 {
                let (want, got) = (want[channel], got[channel]);
                if bitwise {
                    assert_eq!(
                        want.to_bits(),
                        got.to_bits(),
                        "{} on {adapter} at ({x}, {y}), channel {channel}: {want} against {got}",
                        witness.name
                    );
                } else {
                    let tolerance = 1e-4 * want.abs().max(1.0);
                    assert!(
                        (want - got).abs() <= tolerance,
                        "{} on {adapter} at ({x}, {y}), channel {channel}: {want} against {got}",
                        witness.name
                    );
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 0);
    assert!(
        compared_bitwise >= 8,
        "only {compared_bitwise} witnesses took the bitwise path on {adapter}"
    );
}

/// The operators WGSL declines to specify as tightly as IEEE 754 does. A program free of all
/// of them is one the two sides should agree with bit for bit.
fn uses_an_inexact_operator(program: &[quorra_scene::FnOp]) -> bool {
    use quorra_scene::FnOp;
    program.iter().any(|op| {
        matches!(
            op,
            FnOp::Atan
                | FnOp::Cos
                | FnOp::Div
                | FnOp::Exp
                | FnOp::Ln
                | FnOp::Log
                | FnOp::Sin
                | FnOp::Sqrt
        )
    })
}

/// ISO 32000-2 §8.7.4.5.2's discard, on the device rather than only in Rust.
///
/// The domain here is a quarter of the area the points cover — the shape neither of the
/// caller's witnesses has, and the one where a shader that clamped would differ from one that
/// discards. Both halves of the clause are checked: no background leaves the outside at zero
/// coverage, and a background paints exactly itself there.
#[test]
fn the_domain_rule_survives_the_shader() {
    let Some((device, queue, adapter)) = device() else {
        eprintln!("no adapter available; the device half of the function tests did not run");
        return;
    };
    let inside = (0.25_f32, 0.5_f32);
    let outside = (0.75_f32, 0.5_f32);
    // What a shader that clamped into the domain would have produced at `outside`.
    let clamped = [0.5_f32, 0.5, 0.25, 1.0];

    let unpainted = programs::small_domain(None);
    let got = run(&device, &queue, &unpainted, &[inside, outside]);
    assert_eq!(
        got.first().copied(),
        Some([0.25, 0.5, 0.125, 1.0]),
        "{adapter}"
    );
    assert_eq!(got.get(1).copied(), Some([0.0; 4]), "{adapter}");
    assert_ne!(got.get(1).copied(), Some(clamped));

    let background = Color::new(0.1, 0.2, 0.3, 1.0);
    let painted = programs::small_domain(Some(background));
    let got = run(&device, &queue, &painted, &[inside, outside]);
    assert_eq!(got.get(1).copied(), Some([0.1, 0.2, 0.3, 1.0]), "{adapter}");
    assert_ne!(got.get(1).copied(), Some(clamped));
}

/// The three operators whose value the specification states and whose built-ins do not
/// supply it, checked on the device rather than only in Rust — a `ps_round` written correctly
/// and compiled wrongly would pass every host test in the suite.
#[test]
fn the_specified_values_survive_the_shader() {
    use quorra_scene::FnOp;

    let Some((device, queue, adapter)) = device() else {
        eprintln!("no adapter available; the device half of the function tests did not run");
        return;
    };

    let cases: [(&'static str, Vec<FnOp>, f32); 4] = [
        // PLRM3: a tie goes to the greater of the two, so `-6.5 round` is `-6.0`. WGSL's own
        // `round` is half-to-even and Rust's is half-away-from-zero.
        (
            "-6.5 round",
            vec![FnOp::Pop, FnOp::Pop, FnOp::PushReal(-6.5), FnOp::Round],
            -6.0,
        ),
        // The same rule at a positive tie, where half-to-even would give 2.
        (
            "2.5 round",
            vec![FnOp::Pop, FnOp::Pop, FnOp::PushReal(2.5), FnOp::Round],
            3.0,
        ),
        // "Bits shifted out are lost; bits shifted in are 0" — a logical right shift, where
        // an `i32 >>` would sign-extend to -1.
        (
            "-16 -28 bitshift",
            vec![
                FnOp::Pop,
                FnOp::Pop,
                FnOp::PushInt(-16),
                FnOp::PushInt(-28),
                FnOp::Bitshift,
            ],
            15.0,
        ),
        // Table 42's other `not`: one's complement over an integer.
        ("63 not", programs::NOT_ON_INTEGER.to_vec(), -64.0),
    ];

    for (name, program, expected) in cases {
        let witness = programs::witness(&program, programs::WIDE_GRAY);
        let colours = run(&device, &queue, &witness, &[(0.5, 0.5)]);
        assert_eq!(
            colours.first().map(|colour| colour[0]),
            Some(expected),
            "{name} on {adapter}"
        );
    }
}
