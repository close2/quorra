//! The generated function, on the device, at full precision.
//!
//! One responsibility: **run `quorra_function_evaluate` at a list of points and hand back
//! what it computed**, with nothing between the shader and the caller. It is a compute pass
//! over a storage buffer of `vec4<f32>` rather than a raster, and that is the whole design:
//! `doc/spike-function-paint.md` §5 measured 246 044 texels off by one from ADR 0006's 8-bit
//! store *alone*, so a raster would put that conversion between the shader and every
//! assertion and cost the test all of its resolution. `tests/function_lane.rs` is where the
//! store belongs, because there the expectation is a colour.
//!
//! The shader compiled here is the same text `pipeline::function` compiles for a frame,
//! minus `function_lane.wgsl` — the placement, the coverage and the clip, none of which is
//! about the program's arithmetic.

#![allow(
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "test scaffolding whose panic is the calling test failing, and byte offsets \
              over a point list whose length the caller fixes"
)]

use quorra_gpu::function::{background_rgba, generate, range_bounds};
use quorra_gpu::wgpu;
use quorra_scene::{Color, FnOp, FnRange};

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

/// A device to run generated shaders on, and the adapter's name for the messages.
pub struct Compute {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// The adapter this ran on. ADR 0053's consequence — cross-adapter identity is not
    /// promised for this paint — is why every assertion made through this harness names it.
    pub adapter: String,
}

/// What one program is evaluated under: everything `quorra_function_evaluate` takes that is
/// not the point.
pub struct Shading<'a> {
    pub program: &'a [FnOp],
    pub range: FnRange,
    /// §8.7.4.5.2's `Domain` as (min x, max x, min y, max y).
    pub domain: [f32; 4],
    pub background: Option<Color>,
}

impl Compute {
    /// A device, or `None` where this machine has no adapter at all.
    ///
    /// `QUORRA_ADAPTER` picks one by name, as `Options::adapter` does for a `Device`: ADR
    /// 0053 promises no cross-adapter identity for this paint, so a claim made through this
    /// harness is a claim about one adapter and a reader has to be able to choose which.
    #[must_use]
    pub fn new() -> Option<Self> {
        let instance = quorra_gpu::create_instance();
        let wanted = std::env::var("QUORRA_ADAPTER").ok();
        let adapter = match &wanted {
            Some(name) => pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
                .into_iter()
                .find(|adapter| adapter.get_info().name.contains(name.as_str()))?,
            None => pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            }))
            .ok()?,
        };
        let name = adapter.get_info().name;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("quorra function tests"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .ok()?;
        Some(Self {
            device,
            queue,
            adapter: name,
        })
    }

    /// Run one program at every point, returning what the shader computed.
    ///
    /// # Panics
    ///
    /// If the program or its `Range` is not admitted, or the device does not answer the
    /// map: both are the calling test failing rather than a condition to handle.
    #[must_use]
    #[allow(clippy::missing_panics_doc, clippy::cast_possible_truncation)]
    pub fn run(&self, shading: &Shading<'_>, points: &[(f32, f32)]) -> Vec<[f32; 4]> {
        let analysis =
            quorra_gpu::function::analyse(shading.program).expect("the program is admitted");
        let shader = generate(&analysis, shading.range).expect("the Range is admitted");
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("quorra generated function"),
                source: wgpu::ShaderSource::Wgsl(format!("{}{HARNESS}", shader.module()).into()),
            });

        let point_bytes: Vec<u8> = points
            .iter()
            .flat_map(|(x, y)| [x.to_le_bytes(), y.to_le_bytes()])
            .flatten()
            .collect();
        let bound_bytes = bounds_of(shading);
        let output_bytes = (points.len() * 16) as u64;

        let input = self.buffer(&point_bytes, wgpu::BufferUsages::STORAGE);
        let bounds = self.buffer(&bound_bytes, wgpu::BufferUsages::UNIFORM);
        let output = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("function colours"),
            size: output_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("function readback"),
            size: output_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline = self
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("quorra generated function"),
                layout: None,
                module: &module,
                entry_point: Some("evaluate_points"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
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

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(points.len().div_ceil(64) as u32, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output_bytes);
        self.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the device should answer the map");
        let mapped = slice.get_mapped_range().expect("the readback should map");
        let colours = mapped
            .chunks_exact(16)
            .map(|chunk| {
                let component = |index: usize| {
                    let at = index * 4;
                    let bytes = chunk.get(at..at + 4).unwrap_or(&[0; 4]);
                    f32::from_le_bytes(bytes.try_into().unwrap_or([0; 4]))
                };
                [component(0), component(1), component(2), component(3)]
            })
            .collect();
        drop(mapped);
        readback.unmap();
        colours
    }

    fn buffer(&self, bytes: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes.len().max(16) as u64,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, bytes);
        buffer
    }
}

/// The four `vec4<f32>` the emitted function's parameters are packed into, in the order the
/// harness unpacks them: the domain rectangle, the range's low and high bounds, and the
/// background. The padding lanes exist because a uniform array's stride is 16 bytes.
fn bounds_of(shading: &Shading<'_>) -> Vec<u8> {
    let (low, high) = range_bounds(shading.range);
    let background = background_rgba(shading.background);
    let values: [f32; 16] = [
        shading.domain[0],
        shading.domain[1],
        shading.domain[2],
        shading.domain[3],
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
