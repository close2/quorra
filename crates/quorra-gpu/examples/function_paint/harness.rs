//! The instrument: a device, a pipeline, and one full-viewport pass, timed.
//!
//! Deliberately built on raw `wgpu` rather than on [`quorra_gpu::Device`]: this is a
//! spike, and the rule for it is that nothing in the library changes to host it. What
//! it measures is therefore the *paint*, with no scene, no encode and no compositor —
//! which is the right isolation for the question, and a caveat the write-up states.
//!
//! Every duration here is a timestamp query where the adapter has them, because
//! `doc/HANDOVER.md`'s standing trap is that wall clocks lie on this machine. The one
//! wall clock that cannot be avoided is the shader compile, which happens on the host.

use std::time::{Duration, Instant};

/// An adapter, its device, and what a timestamp tick is worth on it.
pub(crate) struct Gpu {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) name: String,
    /// Nanoseconds per timestamp tick; `None` when the adapter has no queries.
    pub(crate) period: Option<f32>,
}

impl Gpu {
    /// Open the first adapter whose name contains `filter`.
    pub(crate) fn open(filter: &str) -> Option<Self> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
            .into_iter()
            .find(|adapter| adapter.get_info().name.contains(filter))?;
        let wanted = wgpu::Features::TIMESTAMP_QUERY;
        let features = adapter.features() & wanted;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("quorra function-paint spike"),
            required_features: features,
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::default(),
        }))
        .ok()?;
        let period = features
            .contains(wanted)
            .then(|| queue.get_timestamp_period());
        Some(Self {
            device,
            queue,
            name: adapter.get_info().name,
            period,
        })
    }
}

/// A compiled paint: the pipeline, and what compiling it cost on the host thread.
pub(crate) struct Paint {
    pub(crate) pipeline: wgpu::RenderPipeline,
    pub(crate) layout: wgpu::BindGroupLayout,
    /// Module creation — the WGSL parse and validation.
    pub(crate) module: Duration,
    /// Pipeline creation — the backend's own compile to machine code.
    pub(crate) link: Duration,
}

/// Build a pipeline from a whole WGSL source, timing the two halves separately.
///
/// The split matters for `PLAN.md` §1.8: a naga parse is portable work this library
/// controls, and a driver compile is not, so a startup number that adds them cannot
/// say which regressed.
pub(crate) fn build(gpu: &Gpu, source: &str, with_program: bool) -> Paint {
    let mut entries = vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }];
    if with_program {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
    }
    let layout = gpu
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("function paint"),
            entries: &entries,
        });
    let pipeline_layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("function paint"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

    let started = Instant::now();
    let shader = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("function paint"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    // `create_shader_module` returns before the backend has seen the module; a poll
    // is what makes the number the parse rather than a queue insertion.
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let module = started.elapsed();

    let started = Instant::now();
    let pipeline = gpu
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("function paint"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let link = started.elapsed();

    Paint {
        pipeline,
        layout,
        module,
        link,
    }
}

/// The target a run draws into, made once and reused across rounds.
pub(crate) struct Canvas {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) width: u32,
    pub(crate) height: u32,
    query: Option<Query>,
}

struct Query {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    map: wgpu::Buffer,
}

impl Canvas {
    pub(crate) fn new(gpu: &Gpu, width: u32, height: u32) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("function paint target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let query = gpu.period.map(|_| Query {
            set: gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("function paint timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolve: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("timestamp resolve"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            map: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("timestamp map"),
                size: 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        });
        Self {
            texture,
            view,
            width,
            height,
            query,
        }
    }
}

/// What one pass cost, by the two clocks that can see it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Timed {
    /// The pass itself, from the adapter's own timestamps. `None` without queries.
    pub(crate) device: Option<Duration>,
    /// Submit to idle, on the host. Includes submission and the wait.
    pub(crate) wall: Duration,
}

/// Draw one full-viewport pass and wait for it.
pub(crate) fn draw(gpu: &Gpu, canvas: &Canvas, paint: &Paint, bind: &wgpu::BindGroup) -> Timed {
    let mut recorder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("function paint"),
        });
    {
        let mut pass = recorder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("function paint"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &canvas.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: canvas
                .query
                .as_ref()
                .map(|q| wgpu::RenderPassTimestampWrites {
                    query_set: &q.set,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&paint.pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    }
    if let Some(q) = &canvas.query {
        recorder.resolve_query_set(&q.set, 0..2, &q.resolve, 0);
        recorder.copy_buffer_to_buffer(&q.resolve, 0, &q.map, 0, 16);
    }
    let started = Instant::now();
    gpu.queue.submit([recorder.finish()]);
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let wall = started.elapsed();

    let device = canvas
        .query
        .as_ref()
        .zip(gpu.period)
        .and_then(|(q, period)| read_pass(gpu, q, period));
    Timed { device, wall }
}

fn read_pass(gpu: &Gpu, query: &Query, period: f32) -> Option<Duration> {
    let slice = query.map.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    receiver.recv().ok()?.ok()?;
    let ticks = {
        let bytes = slice.get_mapped_range().ok()?;
        let tick = |range: std::ops::Range<usize>| -> u64 {
            let eight: [u8; 8] = bytes[range].try_into().unwrap_or([0; 8]);
            u64::from_le_bytes(eight)
        };
        tick(8..16).saturating_sub(tick(0..8))
    };
    query.map.unmap();
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a pass's tick count is far below 2^53; the truncation is of fractional nanoseconds"
    )]
    Some(Duration::from_nanos(
        (ticks as f64 * f64::from(period)) as u64,
    ))
}

/// Copy the target out and hand back its rows, tightly packed as RGBA8.
pub(crate) fn read_pixels(gpu: &Gpu, canvas: &Canvas) -> Vec<u8> {
    let row = canvas
        .width
        .saturating_mul(4)
        .next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let size = u64::from(row).saturating_mul(u64::from(canvas.height));
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("function paint readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut recorder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    recorder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &canvas.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: canvas.width,
            height: canvas.height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([recorder.finish()]);

    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let mut pixels = Vec::new();
    if let (Ok(_), Ok(bytes)) = (receiver.recv(), slice.get_mapped_range()) {
        let width = canvas.width as usize * 4;
        for y in 0..canvas.height as usize {
            let start = y * row as usize;
            pixels.extend_from_slice(&bytes[start..start + width]);
        }
    }
    buffer.unmap();
    pixels
}
