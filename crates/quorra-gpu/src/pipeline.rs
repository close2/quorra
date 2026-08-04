//! Pipelines and shaders: how few, and how late.
//!
//! §7 of the brief makes this module a startup-latency problem before it is a
//! rendering problem. The caller renders page one on its CPU backend *while we
//! initialise*, so what we cost before the first frame is what decides whether the
//! handover is invisible. The rules, from the brief and `doc/PLAN.md` §1.8:
//!
//! - **No pipeline compilation on the critical path of device construction.**
//!   `PipelineStore::new` creates nothing on the GPU; the warm set compiles on a
//!   background thread (`PipelineStore::spawn_warm_up`), and its duration lands in
//!   `StartupTimings::pipeline_compilation`.
//! - **Compiled lazily.** A render that needs a pipeline the warm thread has not
//!   already produced compiles it on the spot — correct frames, sooner, at a one-off
//!   cost the frame's `Timings::phases` names — and every later frame finds it cached.
//! - **Few.** The warm set is two lanes: the analytic rectangle (`Kind::RectOver`)
//!   and the coverage quad (`Kind::CoverOver`) — exactly the "glyph quads,
//!   rectangle fills" the brief's §7 says a page of text needs. Everything else —
//!   knockout variants, the image and shading quads (ADR 0011), the compositor's
//!   passes — compiles on first use. Each kind is instantiated per target format.
//!
//! The pipeline cache blob §7 also asks for is deliberately absent: `wgpu` 30 exposes
//! it only through an `unsafe` constructor, this crate is `#![forbid(unsafe_code)]`,
//! and ADR 0013 weighed the exception against the startup measurement and declined
//! it — the warm set compiles in ~9 ms on a thread nobody blocks on.
//!
//! # Shaders are code, and this project's rules apply to them
//!
//! WGSL lives in `src/shaders/`, and every function implementing a normative
//! requirement carries its clause number in a comment — a WGSL blend function is not
//! exempt from CLAUDE.md principle 5 just because it is not Rust. A shader whose
//! invariants are not stated beside it is write-only code (principle 4); each shader
//! states its coverage definition and its determinism argument inline.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

/// The format the warm-up thread compiles for: the readback and host-texture format,
/// which every headless frame needs.
pub(crate) const WARM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Which pipeline: the two lanes in their three blend styles, plus the compositor's
/// own passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    /// Analytic rectangles, premultiplied over (`rect.wgsl` `fs_main`, ADR 0005).
    RectOver,
    /// Rectangle shape-erase for knockout (`fs_shape`, factors `ZERO`/`1-alpha`).
    RectErase,
    /// Rectangle additive deposit for knockout (`fs_main`, factors `ONE`/`ONE`).
    RectAdd,
    /// Coverage quads, premultiplied over (`coverage.wgsl` `fs_main`).
    CoverOver,
    /// Coverage shape-erase for knockout.
    CoverErase,
    /// Coverage additive deposit for knockout.
    CoverAdd,
    /// One image quad, premultiplied over (`image.wgsl` `fs_main`, ADR 0011).
    ImageOver,
    /// Image shape-erase for knockout (`fs_shape`).
    ImageErase,
    /// Image additive deposit for knockout.
    ImageAdd,
    /// One shading or mesh quad, premultiplied over (`shading.wgsl` `fs_main`,
    /// ISO 32000-2 §8.7.4.5).
    ShadedOver,
    /// Shading shape-erase for knockout (`fs_shape`).
    ShadedErase,
    /// Shading additive deposit for knockout.
    ShadedAdd,
    /// One finished layer onto its backdrop under a §11.3.5 blend mode
    /// (`composite.wgsl`; REPLACE, arithmetic wholly in-shader).
    Composite,
    /// Soft-mask reduction to R8 (`reduce.wgsl`; §11.5, byte-agreed).
    Reduce,
    /// Root layer to target, unchanged (`blit.wgsl`).
    Blit,
    /// Outline triangles accumulating a winding number (`winding.wgsl` `fs_winding`;
    /// additive, ADR 0016). The GPU coverage lane's first pass.
    Winding,
    /// Winding to coverage under a fill rule (`winding.wgsl` `fs_resolve`; REPLACE).
    WindingResolve,
}

/// Everything shared by every pipeline: parsed shaders and the bind-group layouts.
struct Base {
    rect_shader: wgpu::ShaderModule,
    cover_shader: wgpu::ShaderModule,
    image_shader: wgpu::ShaderModule,
    shading_shader: wgpu::ShaderModule,
    composite_shader: wgpu::ShaderModule,
    reduce_shader: wgpu::ShaderModule,
    blit_shader: wgpu::ShaderModule,
    winding_shader: wgpu::ShaderModule,
    globals_layout: wgpu::BindGroupLayout,
    /// Group 1 of both lanes: atlas, scratch, soft mask.
    textures_layout: wgpu::BindGroupLayout,
    /// Image quad: params, image (filterable), sampler, soft mask, scratch.
    image_layout: wgpu::BindGroupLayout,
    /// Shading quad: params, paint (ramp or mesh), scratch, soft mask.
    shading_layout: wgpu::BindGroupLayout,
    /// Composite pass: params, backdrop, src, soft mask, scratch.
    composite_layout: wgpu::BindGroupLayout,
    /// Reduce pass: params (with the transfer table), src.
    reduce_layout: wgpu::BindGroupLayout,
    /// Blit pass: src.
    blit_layout: wgpu::BindGroupLayout,
    /// Winding and resolve passes: the sheet's globals, read by both stages.
    winding_layout: wgpu::BindGroupLayout,
    lane_layout: wgpu::PipelineLayout,
    image_pipe_layout: wgpu::PipelineLayout,
    shading_pipe_layout: wgpu::PipelineLayout,
    composite_pipe_layout: wgpu::PipelineLayout,
    reduce_pipe_layout: wgpu::PipelineLayout,
    blit_pipe_layout: wgpu::PipelineLayout,
    winding_pipe_layout: wgpu::PipelineLayout,
    resolve_pipe_layout: wgpu::PipelineLayout,
}

/// The bind-group layouts alone, grouped so [`PipelineStore::base`] stays a
/// composition of named parts.
struct BindLayouts {
    globals: wgpu::BindGroupLayout,
    textures: wgpu::BindGroupLayout,
    image: wgpu::BindGroupLayout,
    shading: wgpu::BindGroupLayout,
    composite: wgpu::BindGroupLayout,
    reduce: wgpu::BindGroupLayout,
    blit: wgpu::BindGroupLayout,
    winding: wgpu::BindGroupLayout,
}

struct StoreState {
    base: Option<Base>,
    pipelines: HashMap<(Kind, wgpu::TextureFormat), Arc<wgpu::RenderPipeline>>,
    warm: bool,
    warm_duration: Option<Duration>,
}

/// The lazily-populated set of render pipelines, shared between the device and its
/// warm-up thread.
///
/// Compilation happens under the store's one lock, so two threads never compile the
/// same pipeline twice: whoever arrives second blocks briefly and finds it done.
pub(crate) struct PipelineStore {
    device: wgpu::Device,
    state: Mutex<StoreState>,
    warmed: Condvar,
}

impl std::fmt::Debug for PipelineStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.lock();
        f.debug_struct("PipelineStore")
            .field("compiled", &state.pipelines.len())
            .field("warm", &state.warm)
            .finish_non_exhaustive()
    }
}

impl PipelineStore {
    /// A store with nothing compiled. Cheap by design: this runs inside
    /// `Device::headless`, whose contract is to return before pipelines exist.
    pub(crate) fn new(device: wgpu::Device) -> Arc<Self> {
        Arc::new(Self {
            device,
            state: Mutex::new(StoreState {
                base: None,
                pipelines: HashMap::new(),
                warm: false,
                warm_duration: None,
            }),
            warmed: Condvar::new(),
        })
    }

    /// Compile the warm set on a background thread, so device construction returns
    /// first. §2.1 of the brief also says a device must not *require* a background
    /// thread: if the host cannot spawn one, the warm set compiles inline instead —
    /// construction blocks for the compile, which is the documented cost of a host
    /// with no threads to give, and `is_warm` keeps meaning what it says.
    pub(crate) fn spawn_warm_up(self: &Arc<Self>) {
        let store = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("quorra-warm-up".into())
            .spawn(move || store.warm_up_now());
        if spawned.is_err() {
            self.warm_up_now();
        }
        // The JoinHandle is dropped deliberately: completion is observed through
        // `is_warm`/`wait_until_warm`, and rendering does not depend on the thread —
        // a frame that arrives early compiles what it needs on the spot.
    }

    /// The warm-up itself: the two over-lanes a page of text needs (§7 — the
    /// knockout variants, the compositor and the reduction compile on first use).
    fn warm_up_now(&self) {
        let start = Instant::now();
        let (_pipeline, _compiled) = self.get(Kind::RectOver, WARM_FORMAT);
        let (_pipeline, _compiled) = self.get(Kind::CoverOver, WARM_FORMAT);
        let duration = start.elapsed();
        let mut state = self.lock();
        state.warm = true;
        state.warm_duration = Some(duration);
        drop(state);
        self.warmed.notify_all();
    }

    /// The pipeline for a lane and target format, compiling it now if the warm
    /// thread has not already. The second element is `Some(duration)` when this call
    /// did the compiling, so the frame that paid the one-off cost can name it in its
    /// `Timings::phases`.
    pub(crate) fn get(
        &self,
        kind: Kind,
        format: wgpu::TextureFormat,
    ) -> (Arc<wgpu::RenderPipeline>, Option<Duration>) {
        let mut state = self.lock();
        if let Some(pipeline) = state.pipelines.get(&(kind, format)) {
            return (Arc::clone(pipeline), None);
        }
        let start = Instant::now();
        let pipeline = Arc::new(self.compile(&mut state, kind, format));
        state
            .pipelines
            .insert((kind, format), Arc::clone(&pipeline));
        (pipeline, Some(start.elapsed()))
    }

    /// The bind-group layout for the globals uniform.
    pub(crate) fn globals_layout(&self) -> wgpu::BindGroupLayout {
        let mut state = self.lock();
        Self::base(&self.device, &mut state).globals_layout.clone()
    }

    /// The bind-group layout for the lane textures (atlas, scratch, soft mask).
    pub(crate) fn textures_layout(&self) -> wgpu::BindGroupLayout {
        let mut state = self.lock();
        Self::base(&self.device, &mut state).textures_layout.clone()
    }

    /// The image quad's bind-group layout.
    pub(crate) fn image_layout(&self) -> wgpu::BindGroupLayout {
        let mut state = self.lock();
        Self::base(&self.device, &mut state).image_layout.clone()
    }

    /// The shading quad's bind-group layout.
    pub(crate) fn shading_layout(&self) -> wgpu::BindGroupLayout {
        let mut state = self.lock();
        Self::base(&self.device, &mut state).shading_layout.clone()
    }

    /// The composite pass's bind-group layout.
    pub(crate) fn composite_layout(&self) -> wgpu::BindGroupLayout {
        let mut state = self.lock();
        Self::base(&self.device, &mut state)
            .composite_layout
            .clone()
    }

    /// The reduce pass's bind-group layout.
    pub(crate) fn reduce_layout(&self) -> wgpu::BindGroupLayout {
        let mut state = self.lock();
        Self::base(&self.device, &mut state).reduce_layout.clone()
    }

    /// The winding sheet's globals layout, shared by both passes of the GPU lane.
    pub(crate) fn winding_layout(&self) -> wgpu::BindGroupLayout {
        let mut state = self.lock();
        Self::base(&self.device, &mut state).winding_layout.clone()
    }

    /// The blit pass's bind-group layout.
    pub(crate) fn blit_layout(&self) -> wgpu::BindGroupLayout {
        let mut state = self.lock();
        Self::base(&self.device, &mut state).blit_layout.clone()
    }

    /// Whether the warm set — every pipeline a page of text needs — exists.
    pub(crate) fn is_warm(&self) -> bool {
        self.lock().warm
    }

    /// How long the warm set took to compile, once it has.
    pub(crate) fn warm_duration(&self) -> Option<Duration> {
        self.lock().warm_duration
    }

    /// Block until the warm-up reports done. Measurement support: startup numbers
    /// need a defined "fully warm" moment to be comparable.
    pub(crate) fn wait_until_warm(&self) {
        let mut state = self.lock();
        while !state.warm {
            state = self
                .warmed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    fn lock(&self) -> MutexGuard<'_, StoreState> {
        // A poisoned lock means a compile panicked; every mutation under this lock is
        // an atomic insert or flag set, so the guarded data is still consistent and
        // continuing is sound.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The shared base — parsed shaders and layouts — created on first need.
    fn base<'a>(device: &wgpu::Device, state: &'a mut StoreState) -> &'a Base {
        state.base.get_or_insert_with(|| {
            let module = |label: &str, source: &str| {
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(label),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                })
            };
            let layouts = Self::bind_layouts(device);
            let pipe_layout = |label: &str, groups: &[&wgpu::BindGroupLayout]| {
                let refs: Vec<Option<&wgpu::BindGroupLayout>> =
                    groups.iter().map(|g| Some(*g)).collect();
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: &refs,
                    immediate_size: 0,
                })
            };
            let lane_layout = pipe_layout("quorra lane", &[&layouts.globals, &layouts.textures]);
            let image_pipe_layout = pipe_layout("quorra image", &[&layouts.image]);
            let shading_pipe_layout = pipe_layout("quorra shading", &[&layouts.shading]);
            let composite_pipe_layout = pipe_layout("quorra composite", &[&layouts.composite]);
            let reduce_pipe_layout = pipe_layout("quorra reduce", &[&layouts.reduce]);
            let blit_pipe_layout = pipe_layout("quorra blit", &[&layouts.blit]);
            let winding_pipe_layout = pipe_layout("quorra winding", &[&layouts.winding]);
            let resolve_pipe_layout =
                pipe_layout("quorra winding resolve", &[&layouts.winding, &layouts.blit]);

            Base {
                rect_shader: module("quorra rect", include_str!("shaders/rect.wgsl")),
                cover_shader: module("quorra coverage", include_str!("shaders/coverage.wgsl")),
                image_shader: module("quorra image", include_str!("shaders/image.wgsl")),
                shading_shader: module("quorra shading", include_str!("shaders/shading.wgsl")),
                composite_shader: module(
                    "quorra composite",
                    include_str!("shaders/composite.wgsl"),
                ),
                reduce_shader: module("quorra reduce", include_str!("shaders/reduce.wgsl")),
                blit_shader: module("quorra blit", include_str!("shaders/blit.wgsl")),
                winding_shader: module("quorra winding", include_str!("shaders/winding.wgsl")),
                globals_layout: layouts.globals,
                textures_layout: layouts.textures,
                image_layout: layouts.image,
                shading_layout: layouts.shading,
                composite_layout: layouts.composite,
                reduce_layout: layouts.reduce,
                blit_layout: layouts.blit,
                winding_layout: layouts.winding,
                lane_layout,
                image_pipe_layout,
                shading_pipe_layout,
                composite_pipe_layout,
                reduce_pipe_layout,
                blit_pipe_layout,
                winding_pipe_layout,
                resolve_pipe_layout,
            }
        })
    }

    /// Every bind-group layout, in one place: the binding tables the shaders in
    /// `src/shaders/` assume, entry for entry.
    fn bind_layouts(device: &wgpu::Device) -> BindLayouts {
        let uniform_entry =
            |binding: u32, size: u64, visibility: wgpu::ShaderStages| wgpu::BindGroupLayoutEntry {
                binding,
                visibility,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size),
                },
                count: None,
            };
        let texture_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        // The image's texture is the one filterable binding in the crate: linear
        // filtering is the placement's resolved decision (§4.5), so the hardware
        // sampler must be usable on it.
        let filterable_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let quad_uniform = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let make = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        BindLayouts {
            globals: make(
                "quorra globals",
                &[uniform_entry(0, 16, wgpu::ShaderStages::VERTEX)],
            ),
            textures: make(
                "quorra lane sources",
                &[texture_entry(0), texture_entry(1), texture_entry(2)],
            ),
            image: make(
                "quorra image",
                &[
                    uniform_entry(0, 112, quad_uniform),
                    filterable_entry(1),
                    sampler_entry(2),
                    texture_entry(3),
                    texture_entry(4),
                ],
            ),
            shading: make(
                "quorra shading",
                &[
                    uniform_entry(0, 144, quad_uniform),
                    texture_entry(1),
                    texture_entry(2),
                    texture_entry(3),
                ],
            ),
            composite: make(
                "quorra composite",
                &[
                    uniform_entry(0, 64, wgpu::ShaderStages::FRAGMENT),
                    texture_entry(1),
                    texture_entry(2),
                    texture_entry(3),
                    texture_entry(4),
                ],
            ),
            reduce: make(
                "quorra reduce",
                &[
                    uniform_entry(0, 288, wgpu::ShaderStages::FRAGMENT),
                    texture_entry(1),
                ],
            ),
            blit: make("quorra blit", &[texture_entry(0)]),
            // Both stages read it: the vertex stage for the sheet's size, the resolve
            // fragment for the fill rule and the sample grid.
            // 32 bytes: the sheet size, this draw's sample offset, and the channel
            // mask it accumulates into.
            winding: make("quorra winding", &[uniform_entry(0, 32, quad_uniform)]),
        }
    }

    // One match arm per pipeline kind: irreducible dispatch, each arm a handful of
    // lines (clippy.toml's stated exception style).
    #[allow(clippy::too_many_lines)]
    fn compile(
        &self,
        state: &mut StoreState,
        kind: Kind,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        struct Spec<'a> {
            label: &'a str,
            shader: &'a wgpu::ShaderModule,
            layout: &'a wgpu::PipelineLayout,
            /// The vertex entry point. Every lane shares `vs_main`; the winding pass
            /// has two stages of its own in one module.
            vertex: &'a str,
            entry: &'a str,
            blend: Option<wgpu::BlendState>,
            buffer: Option<(u64, &'a [wgpu::VertexAttribute])>,
            /// How the buffer advances. The lanes draw one quad per instance; the
            /// winding pass draws real triangles, one vertex at a time.
            step: wgpu::VertexStepMode,
            /// Four-corner strip quads; `false` for the full-screen triangle passes.
            strip: bool,
        }
        let base = Self::base(&self.device, state);
        // Premultiplied over: (ONE, ONE_MINUS_SRC_ALPHA). Knockout erase scales the
        // backdrop by (1 − shape): (ZERO, ONE_MINUS_SRC_ALPHA) with shape in the
        // source alpha. Knockout add deposits shape·element: (ONE, ONE). ADR 0010.
        let component = |src, dst| wgpu::BlendComponent {
            src_factor: src,
            dst_factor: dst,
            operation: wgpu::BlendOperation::Add,
        };
        let both = |src, dst| wgpu::BlendState {
            color: component(src, dst),
            alpha: component(src, dst),
        };
        let over = both(wgpu::BlendFactor::One, wgpu::BlendFactor::OneMinusSrcAlpha);
        let erase = both(wgpu::BlendFactor::Zero, wgpu::BlendFactor::OneMinusSrcAlpha);
        let add = both(wgpu::BlendFactor::One, wgpu::BlendFactor::One);

        let rect_attributes = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4];
        let resolve_attributes = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x2];
        let winding_attributes =
            wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];
        let cover_attributes = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x2, 2 => Float32x4,
            3 => Float32x4, 4 => Float32x4
        ];
        let spec = match kind {
            Kind::RectOver | Kind::RectErase | Kind::RectAdd => Spec {
                label: "quorra rect",
                shader: &base.rect_shader,
                layout: &base.lane_layout,
                vertex: "vs_main",
                entry: if kind == Kind::RectErase {
                    "fs_shape"
                } else {
                    "fs_main"
                },
                blend: Some(match kind {
                    Kind::RectErase => erase,
                    Kind::RectAdd => add,
                    _ => over,
                }),
                buffer: Some((crate::encode::RECT_INSTANCE_STRIDE, &rect_attributes)),
                step: wgpu::VertexStepMode::Instance,
                strip: true,
            },
            Kind::CoverOver | Kind::CoverErase | Kind::CoverAdd => Spec {
                label: "quorra coverage",
                shader: &base.cover_shader,
                layout: &base.lane_layout,
                vertex: "vs_main",
                entry: if kind == Kind::CoverErase {
                    "fs_shape"
                } else {
                    "fs_main"
                },
                blend: Some(match kind {
                    Kind::CoverErase => erase,
                    Kind::CoverAdd => add,
                    _ => over,
                }),
                buffer: Some((crate::encode::QUAD_INSTANCE_STRIDE, &cover_attributes)),
                step: wgpu::VertexStepMode::Instance,
                strip: true,
            },
            Kind::ImageOver | Kind::ImageErase | Kind::ImageAdd => Spec {
                label: "quorra image",
                shader: &base.image_shader,
                layout: &base.image_pipe_layout,
                vertex: "vs_main",
                entry: if kind == Kind::ImageErase {
                    "fs_shape"
                } else {
                    "fs_main"
                },
                blend: Some(match kind {
                    Kind::ImageErase => erase,
                    Kind::ImageAdd => add,
                    _ => over,
                }),
                buffer: None,
                step: wgpu::VertexStepMode::Instance,
                strip: true,
            },
            Kind::ShadedOver | Kind::ShadedErase | Kind::ShadedAdd => Spec {
                label: "quorra shading",
                shader: &base.shading_shader,
                layout: &base.shading_pipe_layout,
                vertex: "vs_main",
                entry: if kind == Kind::ShadedErase {
                    "fs_shape"
                } else {
                    "fs_main"
                },
                blend: Some(match kind {
                    Kind::ShadedErase => erase,
                    Kind::ShadedAdd => add,
                    _ => over,
                }),
                buffer: None,
                step: wgpu::VertexStepMode::Instance,
                strip: true,
            },
            Kind::Composite => Spec {
                label: "quorra composite",
                shader: &base.composite_shader,
                layout: &base.composite_pipe_layout,
                vertex: "vs_main",
                entry: "fs_main",
                blend: None, // REPLACE: the shader computes §11.3.6 wholly itself
                buffer: None,
                step: wgpu::VertexStepMode::Instance,
                strip: false,
            },
            Kind::Reduce => Spec {
                label: "quorra reduce",
                shader: &base.reduce_shader,
                layout: &base.reduce_pipe_layout,
                vertex: "vs_main",
                entry: "fs_main",
                blend: None,
                buffer: None,
                step: wgpu::VertexStepMode::Instance,
                strip: false,
            },
            Kind::Blit => Spec {
                label: "quorra blit",
                shader: &base.blit_shader,
                layout: &base.blit_pipe_layout,
                vertex: "vs_main",
                entry: "fs_main",
                blend: None,
                buffer: None,
                step: wgpu::VertexStepMode::Instance,
                strip: false,
            },
            // Additive, and that is the whole mechanism: what lands in the sheet is
            // the sum of every triangle's ±1, which is the winding number (ADR 0016).
            Kind::Winding => Spec {
                label: "quorra winding",
                shader: &base.winding_shader,
                layout: &base.winding_pipe_layout,
                vertex: "vs_winding",
                entry: "fs_winding",
                blend: Some(add),
                buffer: Some((crate::outline::WindingVertex::STRIDE, &winding_attributes)),
                step: wgpu::VertexStepMode::Vertex,
                strip: false,
            },
            // Additive as well, for a different reason: each pass carries four of the
            // frame's samples and the sheet sums the groups.
            Kind::WindingResolve => Spec {
                label: "quorra winding resolve",
                shader: &base.winding_shader,
                layout: &base.resolve_pipe_layout,
                vertex: "vs_resolve",
                entry: "fs_resolve",
                blend: Some(add),
                buffer: Some((crate::winding::TILE_STRIDE, &resolve_attributes)),
                step: wgpu::VertexStepMode::Instance,
                strip: true,
            },
        };
        let buffers: Vec<Option<wgpu::VertexBufferLayout<'_>>> = spec
            .buffer
            .map(|(stride, attributes)| {
                vec![Some(wgpu::VertexBufferLayout {
                    array_stride: stride,
                    step_mode: spec.step,
                    attributes,
                })]
            })
            .unwrap_or_default();
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(spec.label),
                layout: Some(spec.layout),
                vertex: wgpu::VertexState {
                    module: spec.shader,
                    entry_point: Some(spec.vertex),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: if spec.strip {
                        wgpu::PrimitiveTopology::TriangleStrip
                    } else {
                        wgpu::PrimitiveTopology::TriangleList
                    },
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: spec.shader,
                    entry_point: Some(spec.entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: spec.blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }
}
