//! The device: adapter, queue, pipelines, and the frames they produce.
//!
//! # The two rules that outrank everything else here
//!
//! **A frame is drawn, or it is refused. There is no third state.** Every allocation
//! in a frame is sized by counting first (`encode.rs`) and checked against a stated
//! budget; every refusal is an [`Err`] naming what was exceeded or unavailable
//! ([`crate::error`]); and a [`Frame`] is constructed only after every fallible step
//! has succeeded, so a failed frame cannot report itself drawn.
//!
//! **Startup is a first-class requirement** (§2.1, §7 of the brief). Construction
//! blocks on adapter selection and device creation — the two things a device *is* —
//! and on nothing else: pipelines compile on a background thread
//! ([`Device::is_warm`]), and [`Device::headless`] is callable from any thread while
//! requiring none. What construction cost is reported by [`Device::startup`], split
//! into the three numbers §7 names, because a regression that cannot be attributed can
//! only be argued about.
//!
//! The neighbours own the rest: [`crate::pipeline`] the shaders and their laziness,
//! `surface.rs` tier 2's lifecycle, `readback.rs` tier 1's price, [`crate::error`]
//! the refusals.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use quorra_scene::Scene;

use quorra_scene::{
    Color, ImageId, ImageSpec, MeshId, MeshSpec, OutlineId, RampId, ResourceId, Segment, Stop,
};

use crate::atlas::AtlasStore;
use crate::compose::{self, Executor};
use crate::encode::{self, ChildOp, Encoded, ImageOp, MaskPlan, PaintSource, ShadedOp};
use crate::error::{DeviceError, RenderError};
use crate::frame::{Counters, Frame, Payload, Raster, TimingProvenance, Timings};
use crate::pipeline::{PipelineStore, WARM_FORMAT};
use crate::readback;
use crate::report::{Report, ReportKind};
use crate::resources::ResourceStore;
use crate::surface::SurfaceState;
use crate::target::Target;
pub(crate) use crate::timing::PassQuery;
use crate::timing::{self, TimestampSupport};
use crate::viewport::Viewport;

/// The default per-frame budget for scene-derived allocations, in bytes.
///
/// A stated number rather than an implicit one, because a GPU buffer sized from
/// document-derived arithmetic is a decompression bomb with a different name
/// (CLAUDE.md principle 3). 256 MiB of instance data is roughly eight million
/// rectangle commands — beyond any real page by orders of magnitude, while still
/// refusing runaway input long before an allocator does.
pub const DEFAULT_MAX_FRAME_BYTES: u64 = 256 * 1024 * 1024;

/// The default budget for resident resources — outlines, images, ramps, meshes — in
/// bytes.
///
/// The same principle as [`DEFAULT_MAX_FRAME_BYTES`], at resource scope: §4.7's
/// 60 000×60 000 image arrives from real files by way of a correct interpreter, and
/// its 14.4 GB of RGBA8 must be a refusal naming this number, never an allocation
/// attempt. 512 MiB holds every page of every corpus document the brief quotes with
/// room to spare, and the caller can raise it deliberately.
pub const DEFAULT_MAX_RESOURCE_BYTES: u64 = 512 * 1024 * 1024;

/// The default glyph-atlas budget, in bytes (an R8 texel is one byte).
///
/// 8 MiB holds roughly two thousand 64×64 tiles — far beyond the 107 distinct
/// outlines of the brief's dense page at several phases each — while staying an
/// afterthought next to a single 1191×1684 target. §6.3: the atlas is sized from a
/// budget the caller sets, never from a constant of ours alone; this is only the
/// default.
pub const DEFAULT_ATLAS_BUDGET: u64 = 8 * 1024 * 1024;

/// The default sub-pixel quantum of the glyph cache: 1/16 of a pixel.
///
/// §4.5's fifth decision, measured by the caller (its ADR 0131): 1/16 reused 5.0× on
/// a dense page and left its oracle's verdicts unmoved; 1/8 contradicted pages.
pub const DEFAULT_GLYPH_QUANTUM: u16 = 16;

/// Construction options.
///
/// M4 adds the glyph-cache quantum and the atlas budget here — the fifth decision of
/// §4.5, the one that is the caller's to make and ours to expose. M8 revisits the
/// pipeline-cache blob (see `doc/PLAN.md`).
#[derive(Debug, Clone)]
pub struct Options {
    /// Select the adapter by case-insensitive substring of its name — `"llvmpipe"`
    /// picks the software rasteriser, `"radv"` the Radeon Vulkan driver. `None` asks
    /// wgpu for the highest-performance adapter it can find. Ties among matches are
    /// broken by name order, so the same request on the same machine picks the same
    /// adapter (§4.6's spirit applied to setup).
    pub adapter: Option<String>,
    /// The per-frame budget for scene-derived allocations, in bytes. Exceeding it is
    /// a [`RenderError::FrameBudgetExceeded`] naming both numbers, before anything is
    /// allocated.
    pub max_frame_bytes: u64,
    /// The budget for resident resources (outlines, images, ramps, meshes), in bytes.
    /// Exceeding it is a [`DeviceError::ResourceBudgetExceeded`] naming all three
    /// numbers, before anything is stored.
    pub max_resource_bytes: u64,
    /// The glyph atlas budget, in bytes ([`DEFAULT_ATLAS_BUDGET`]).
    pub atlas_budget: u64,
    /// The sub-pixel quantum of the glyph cache, as a denominator: `Some(16)` keys
    /// glyph tiles at 1/16-pixel phases; `None` switches quantisation **off** (exact
    /// phase keying — correct everywhere, and it almost never hits, which is the
    /// caller's own measurement). Quantising moves rendered text by at most half a
    /// quantum, which is why this is exposed rather than chosen silently (§4.5).
    pub glyph_quantum: Option<u16>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            adapter: None,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            max_resource_bytes: DEFAULT_MAX_RESOURCE_BYTES,
            atlas_budget: DEFAULT_ATLAS_BUDGET,
            glyph_quantum: Some(DEFAULT_GLYPH_QUANTUM),
        }
    }
}

/// What this adapter can actually do, discoverable before any frame (§5 of the brief:
/// a limit that must exist is discoverable, through this and `Scene::cost`).
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum width or height of a render target, in pixels.
    pub max_target_size: u32,
    /// The per-frame budget for scene-derived allocations, as configured.
    pub max_frame_bytes: u64,
    /// The resident-resource budget, as configured.
    pub max_resource_bytes: u64,
}

/// What startup cost, split into the three phases §7 names, so a regression can be
/// attributed rather than argued about. Gated in CI from the commit that first
/// produced it.
#[derive(Debug, Clone, Copy)]
pub struct StartupTimings {
    /// Instance creation and adapter enumeration/selection.
    pub adapter_enumeration: Duration,
    /// `request_device`: getting a queue on the chosen adapter.
    pub device_creation: Duration,
    /// Compiling the warm pipeline set, on the background thread. `None` while that
    /// is still in flight — poll [`Device::is_warm`], or read this again later.
    pub pipeline_compilation: Option<Duration>,
}

/// The rendering device: an adapter, a queue, and the pipelines a scene needs.
///
/// Constructible on a background thread and not requiring one (§2.1). Headless is the
/// first-class form — it is what the caller's test suite and correctness oracle use.
#[derive(Debug)]
pub struct Device {
    gpu: wgpu::Device,
    queue: wgpu::Queue,
    description: String,
    limits: Limits,
    pipelines: Arc<PipelineStore>,
    resources: ResourceStore,
    atlas: AtlasStore,
    atlas_texture: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// Device-resident forms of uploaded paints, realised lazily on first use
    /// (M2 owns the validated CPU copy; this lane owns the bytes on the GPU).
    image_textures: HashMap<u32, (wgpu::Texture, wgpu::TextureView)>,
    ramp_textures: HashMap<u32, (wgpu::Texture, wgpu::TextureView)>,
    mesh_textures: HashMap<u32, (wgpu::Texture, wgpu::TextureView)>,
    /// The one filtering sampler (clamp-to-edge linear), for `ImageFilter::Linear`.
    linear_sampler: wgpu::Sampler,
    dummy_texture: Option<wgpu::TextureView>,
    glyph_quantum: Option<u16>,
    timestamps: Option<TimestampSupport>,
    surface: Option<SurfaceState>,
    startup_adapter_enumeration: Duration,
    startup_device_creation: Duration,
}

/// What a render pass draws into for one frame.
enum Bound<'a> {
    /// A texture this frame created (`Target::Readback`).
    Owned(wgpu::Texture),
    /// The caller's texture (`Target::Texture`).
    Borrowed(&'a wgpu::Texture),
    /// The acquired swapchain texture (`Target::Surface`).
    Acquired(wgpu::SurfaceTexture),
}

impl Bound<'_> {
    fn texture(&self) -> &wgpu::Texture {
        match self {
            Bound::Owned(t) => t,
            Bound::Borrowed(t) => t,
            Bound::Acquired(s) => &s.texture,
        }
    }
}

/// One frame's per-pass durations and one-off costs.
type FramePhases = Vec<(&'static str, Duration)>;

/// How one frame treats the viewport's damage list (ADR 0012).
enum DamagePlan {
    /// Redraw everything: empty damage, or a target with no retained contents.
    Full,
    /// Render internally, scissored to `bbox`, and patch exactly `rects` onto the
    /// caller's texture — both as `[x, y, width, height]` in target pixels.
    Patch {
        bbox: [u32; 4],
        rects: Vec<[u32; 4]>,
    },
}

/// Phase 2's product: the frame's buffers and textures, scheduled for upload.
struct Upload {
    globals: wgpu::Buffer,
    /// `None` for a lane with nothing to draw — wgpu is never handed a zero-length
    /// buffer (§5: the `debug_layers` lesson).
    rect_instances: Option<wgpu::Buffer>,
    quad_instances: Option<wgpu::Buffer>,
    /// The frame's scratch coverage texture, kept alive until the submit.
    scratch_view: Option<(wgpu::Texture, wgpu::TextureView)>,
    bytes: u64,
    time: Duration,
}

impl Device {
    /// A headless device: no window, no surface. The form the caller's test suite and
    /// oracle use, and the form developed first.
    ///
    /// Returns as soon as the adapter and device exist; pipelines compile in the
    /// background ([`Device::is_warm`] says whether they are done). Callable from any
    /// thread; requires none.
    ///
    /// # Errors
    ///
    /// [`DeviceError::NoAdapter`] when nothing matches [`Options::adapter`] (the
    /// error lists what was available), and [`DeviceError::DeviceCreation`] when the
    /// adapter refuses a device.
    pub fn headless(options: &Options) -> Result<Self, DeviceError> {
        let started = Instant::now();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        Self::build(&instance, None, options, started)
    }

    /// A device that presents to a window. `raw-window-handle` and nothing more
    /// specific: any window type convertible to a [`wgpu::SurfaceTarget`] — which
    /// `wgpu` provides for anything implementing the `raw-window-handle` traits it
    /// re-exports as [`wgpu::rwh`] (integration note 4 in `doc/PLAN.md`).
    ///
    /// # Errors
    ///
    /// Everything [`Device::headless`] can return, plus
    /// [`DeviceError::SurfaceCreation`] when the handle cannot become a surface and
    /// [`DeviceError::SurfaceUnsupported`] when the adapter cannot present to it.
    pub fn for_surface(
        window: impl Into<wgpu::SurfaceTarget<'static>>,
        options: &Options,
    ) -> Result<Self, DeviceError> {
        let started = Instant::now();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window)
            .map_err(|source| DeviceError::SurfaceCreation { source })?;
        Self::build(&instance, Some(surface), options, started)
    }

    /// The names of every adapter wgpu can see on this machine, for choosing an
    /// [`Options::adapter`] filter — and for the cross-adapter byte-equality gate,
    /// which renders on all of them (§4.6, §11.4).
    #[must_use]
    pub fn adapter_names() -> Vec<String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
            .iter()
            .map(|adapter| adapter.get_info().name)
            .collect()
    }

    fn build(
        instance: &wgpu::Instance,
        surface: Option<wgpu::Surface<'static>>,
        options: &Options,
        started: Instant,
    ) -> Result<Self, DeviceError> {
        let adapter = Self::select_adapter(instance, surface.as_ref(), options)?;
        let startup_adapter_enumeration = started.elapsed();

        let info = adapter.get_info();
        let description = format!("{} ({:?}, {:?})", info.name, info.device_type, info.backend);

        let surface_state = surface
            .map(|surface| SurfaceState::new(surface, &adapter, &info.name))
            .transpose()?;

        // Timestamp queries are the difference between measuring §11.1 and inferring
        // it; taken when the adapter offers them, worked around (and said so) when not.
        let wanted = wgpu::Features::TIMESTAMP_QUERY;
        let required_features = adapter.features() & wanted;

        let device_started = Instant::now();
        let (gpu, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("quorra"),
            required_features,
            // The adapter's own limits, not WebGPU's portable defaults: a document
            // renderer wants the real maximum target size, and `Device::limits`
            // reports what was actually obtained.
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|source| DeviceError::DeviceCreation {
            adapter: info.name.clone(),
            source,
        })?;
        let startup_device_creation = device_started.elapsed();

        let timestamps = required_features
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then(|| TimestampSupport {
                period: queue.get_timestamp_period(),
            });

        let pipelines = PipelineStore::new(gpu.clone());
        pipelines.spawn_warm_up();

        // A sampler is a descriptor, not a compilation: creating it here costs
        // startup nothing measurable and spares every frame an Option.
        let linear_sampler = gpu.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("quorra image sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let max_dimension = gpu.limits().max_texture_dimension_2d;
        let limits = Limits {
            max_target_size: max_dimension,
            max_frame_bytes: options.max_frame_bytes,
            max_resource_bytes: options.max_resource_bytes,
        };

        Ok(Self {
            gpu,
            queue,
            description,
            limits,
            pipelines,
            resources: ResourceStore::new(options.max_resource_bytes),
            atlas: AtlasStore::new(options.atlas_budget, max_dimension),
            atlas_texture: None,
            image_textures: HashMap::new(),
            ramp_textures: HashMap::new(),
            mesh_textures: HashMap::new(),
            linear_sampler,
            dummy_texture: None,
            glyph_quantum: options.glyph_quantum,
            timestamps,
            surface: surface_state,
            startup_adapter_enumeration,
            startup_device_creation,
        })
    }

    fn select_adapter(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'static>>,
        options: &Options,
    ) -> Result<wgpu::Adapter, DeviceError> {
        match &options.adapter {
            Some(pattern) => {
                let adapters =
                    pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
                let available: Vec<String> = adapters.iter().map(|a| a.get_info().name).collect();
                let needle = pattern.to_lowercase();
                let mut matches: Vec<wgpu::Adapter> = adapters
                    .into_iter()
                    .filter(|a| a.get_info().name.to_lowercase().contains(&needle))
                    .collect();
                matches.sort_by_key(|a| a.get_info().name);
                matches
                    .into_iter()
                    .next()
                    .ok_or_else(|| DeviceError::NoAdapter {
                        requested: Some(pattern.clone()),
                        available,
                    })
            }
            None => pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: surface,
                ..Default::default()
            }))
            .map_err(|_| {
                let available =
                    pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
                        .iter()
                        .map(|a| a.get_info().name)
                        .collect();
                DeviceError::NoAdapter {
                    requested: None,
                    available,
                }
            }),
        }
    }

    /// The adapter's name, type and backend — for reports and golden-file metadata.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The underlying `wgpu` device and queue.
    ///
    /// For a tier-3 host ([`Target::Texture`]): the texture it hands to
    /// [`Device::render`] must be created from exactly this device, and this is where
    /// it gets one — along with the queue it will composite with. The handles are the
    /// same reference-counted objects this `Device` renders with, not copies.
    #[must_use]
    pub fn wgpu(&self) -> (&wgpu::Device, &wgpu::Queue) {
        (&self.gpu, &self.queue)
    }

    /// What this adapter can actually do, for comparison against `Scene::cost`
    /// *before* a frame is attempted (§5's second preference).
    #[must_use]
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Upload an outline: validated, priced against the resource budget, resident
    /// until [`Device::release`]. The id is what a scene's `fill`/`stroke`/`clip`
    /// reference — uploaded once, referenced many times (§2.2 of the brief: the
    /// caller keys these by `Arc::as_ptr` identity, so a zoom re-uploads nothing).
    ///
    /// # Errors
    ///
    /// [`DeviceError::InvalidResource`] naming what §4.7 refused, or
    /// [`DeviceError::ResourceBudgetExceeded`] naming all three numbers.
    pub fn upload_outline(&mut self, path: &[Segment]) -> Result<OutlineId, DeviceError> {
        self.resources.upload_outline(path)
    }

    /// Upload a decoded image (straight-alpha RGBA8; the filtering decision arrives
    /// per placement on the command, M7 — integration note 1 in `doc/PLAN.md`).
    ///
    /// # Errors
    ///
    /// As [`Device::upload_outline`].
    pub fn upload_image(&mut self, image: &ImageSpec) -> Result<ImageId, DeviceError> {
        self.resources.upload_image(image)
    }

    /// Upload a colour ramp for the shadings of ISO 32000-2 §8.7.4.5 (drawn from M7).
    ///
    /// # Errors
    ///
    /// As [`Device::upload_outline`].
    pub fn upload_ramp(&mut self, stops: &[Stop]) -> Result<RampId, DeviceError> {
        self.resources.upload_ramp(stops)
    }

    /// Upload a pre-rasterised mesh (the caller's `MeshRaster`; integration note 5 —
    /// device-resolution by its design, so a zoom re-uploads meshes).
    ///
    /// # Errors
    ///
    /// As [`Device::upload_outline`].
    pub fn upload_mesh(&mut self, mesh: &MeshSpec) -> Result<MeshId, DeviceError> {
        self.resources.upload_mesh(mesh)
    }

    /// Release a resource and return its bytes to the budget.
    ///
    /// # Errors
    ///
    /// [`DeviceError::UnknownResource`] for an id this device never issued or already
    /// released — an error rather than a no-op, because a double release is a caller
    /// bug and hiding it would hide the defect (integration note 7 in `doc/PLAN.md`).
    pub fn release(&mut self, id: impl Into<ResourceId>) -> Result<(), DeviceError> {
        let id = id.into();
        self.resources.release(id)?;
        // The device-resident form goes with the CPU copy, so the budget's word
        // stays true on the GPU side too.
        match id {
            ResourceId::Image(ImageId(raw)) => {
                self.image_textures.remove(&raw);
            }
            ResourceId::Ramp(RampId(raw)) => {
                self.ramp_textures.remove(&raw);
            }
            ResourceId::Mesh(MeshId(raw)) => {
                self.mesh_textures.remove(&raw);
            }
            ResourceId::Outline(_) => {}
        }
        Ok(())
    }

    /// Bytes currently resident across all uploaded resources, against
    /// [`Limits::max_resource_bytes`].
    #[must_use]
    pub fn resource_bytes_in_use(&self) -> u64 {
        self.resources.in_use_bytes()
    }

    /// Whether every pipeline of the warm set exists. A device that is not yet warm
    /// renders correctly and compiles what it needs on demand; a caller handing over
    /// from a CPU backend may prefer to wait for `true`.
    #[must_use]
    pub fn is_warm(&self) -> bool {
        self.pipelines.is_warm()
    }

    /// Block until the warm set is compiled. Startup measurement support; a caller
    /// that does not care never needs to call it.
    pub fn wait_until_warm(&self) {
        self.pipelines.wait_until_warm();
    }

    /// What startup cost, split three ways (§7). `pipeline_compilation` is `None`
    /// until the background warm-up finishes.
    #[must_use]
    pub fn startup(&self) -> StartupTimings {
        StartupTimings {
            adapter_enumeration: self.startup_adapter_enumeration,
            device_creation: self.startup_device_creation,
            pipeline_compilation: self.pipelines.warm_duration(),
        }
    }

    /// Render one frame of `scene` at `viewport` into `target`.
    ///
    /// The scene is not consumed and carries no target knowledge: the same scene
    /// renders at any number of viewports (§2.3).
    ///
    /// # Errors
    ///
    /// A refused frame is an `Err` naming what was refused — see [`RenderError`]'s
    /// variants. On `Err`, nothing was presented and no pixels are claimed drawn.
    // Taking `Target` by value is §2.4's signature: a target is a discriminant plus a
    // borrow, and a caller-side `&Target` would only add a level of indirection.
    #[allow(clippy::needless_pass_by_value)]
    pub fn render(
        &mut self,
        scene: &Scene,
        viewport: &Viewport<'_>,
        into: Target<'_>,
    ) -> Result<Frame, RenderError> {
        self.validate_viewport(viewport)?;

        let mut reports = Vec::new();
        let damage = Self::plan_damage(viewport, &into, &mut reports)?;

        // Phase 1: classify, rasterise coverage, and count (encode.rs). Runs before
        // any allocation and regardless of target size, so refusals are identical
        // across targets.
        let encode_started = Instant::now();
        let encoded = encode::encode(
            scene,
            viewport,
            self.limits.max_frame_bytes,
            self.limits.max_target_size,
            &self.resources,
            &mut self.atlas,
            self.glyph_quantum,
        )?;
        let encode_time = encode_started.elapsed();

        if viewport.width == 0 || viewport.height == 0 {
            return Self::zero_size_frame(viewport, &into, &encoded, encode_time, reports);
        }

        // Phase 2: allocate (sized by phase 1) and schedule uploads — including
        // the device-resident form of any image, ramp or mesh drawn for the first
        // time this frame.
        let mut encoded = encoded;
        let paint_started = Instant::now();
        let paint_bytes = self.ensure_paint_textures(&encoded)?;
        let paint_time = paint_started.elapsed();
        let upload = self.upload(&mut encoded, viewport);
        let upload_time = upload.time.saturating_add(paint_time);
        let upload_bytes = upload.bytes.saturating_add(paint_bytes);

        // Bind the target, and price the compositor's internal textures before any
        // of them exist (§5: count then allocate; the refusal names both numbers).
        // A patched frame renders through the root pair even when flat.
        let bound = self.bind_target(&into, viewport)?;
        let patches = matches!(&damage, DamagePlan::Patch { rects, .. } if !rects.is_empty());
        let internal_bytes =
            compose::internal_texture_bytes(&encoded, viewport.width, viewport.height, patches);
        if internal_bytes > self.limits.max_frame_bytes {
            return Err(RenderError::FrameBudgetExceeded {
                needed: internal_bytes,
                budget: self.limits.max_frame_bytes,
            });
        }

        let query = self.make_pass_query();
        let (execute_wall, mut phases) =
            self.run_frame(&encoded, &bound, upload, query.as_ref(), &damage)?;

        // Present before reading instrumentation back: the person sees the frame at
        // the earliest moment, the numbers arrive a map later.
        let mut readback_source: Option<wgpu::Texture> = None;
        match bound {
            Bound::Acquired(surface_texture) => self.queue.present(surface_texture),
            Bound::Owned(texture) => readback_source = Some(texture),
            Bound::Borrowed(_) => {}
        }
        let (execute, provenance) = timing::read_pass(
            &self.gpu,
            self.timestamps,
            query.as_ref(),
            execute_wall,
            "content pass",
            &mut phases,
        )?;

        // Phase 4: resolve. Only Readback pays anything here (§6.1: this is the cost
        // that dominated the old backend's offscreen frame, priced separately so
        // §11.1 finally has its answer).
        let (payload, readback) = match readback_source {
            Some(texture) => {
                let readback_started = Instant::now();
                let raster = readback::read_back(
                    &self.gpu,
                    &self.queue,
                    &texture,
                    viewport.width,
                    viewport.height,
                    self.limits.max_target_size,
                )?;
                (Payload::Raster(raster), readback_started.elapsed())
            }
            None => (Payload::None, Duration::ZERO),
        };

        let result = Ok(Frame {
            timings: Timings {
                encode: encode_time,
                upload: upload_time,
                execute,
                readback,
                execute_provenance: provenance,
                phases,
            },
            counters: Counters {
                commands: encoded.commands,
                clip_distinct_regions: encoded.clip_distinct_regions,
                distinct_outlines: encoded.distinct_outlines,
                atlas_entries: u32::try_from(self.atlas.entry_count()).unwrap_or(u32::MAX),
                atlas_distinct_keys: encoded.atlas_distinct_keys,
                segments: encoded.segments,
                bytes_uploaded: upload_bytes,
                ..Counters::default()
            },
            reports,
            payload,
        });
        // A tile fell through to scratch this frame: repack the atlas from empty on
        // the next one, so the working set settles rather than thrashing.
        if encoded.atlas_pressure {
            self.atlas.reset();
        }
        result
    }

    /// Phase 3: the whole device side of one frame — mask realisation, layers,
    /// composites, the flat fast path, timestamps — recorded and submitted.
    fn run_frame(
        &mut self,
        encoded: &Encoded,
        bound: &Bound<'_>,
        upload: Upload,
        query: Option<&PassQuery>,
        damage: &DamagePlan,
    ) -> Result<(Duration, FramePhases), RenderError> {
        let width = bound.texture().width();
        let height = bound.texture().height();
        let mut recorder = self
            .gpu
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("quorra frame"),
            });
        let flat = Executor::is_flat(encoded);
        let patch = match damage {
            DamagePlan::Patch { bbox, rects } if !rects.is_empty() => Some((*bbox, rects)),
            _ => None,
        };
        let pairs = if flat && patch.is_none() {
            Vec::new()
        } else {
            (0..=encoded.layers.len())
                .map(|_| {
                    [
                        self.create_internal_texture("quorra layer", width, height, WARM_FORMAT),
                        self.create_internal_texture("quorra layer", width, height, WARM_FORMAT),
                    ]
                })
                .collect()
        };
        let dummy_view = self.ensure_dummy();
        let mask_count = encoded.mask_plans.len();
        let mut executor = Executor {
            device: self,
            encoded,
            width,
            height,
            pairs,
            mask_views: (0..mask_count).map(|_| None).collect(),
            rect_buffer: upload.rect_instances,
            quad_buffer: upload.quad_instances,
            globals_bind: self.bind_globals(&upload.globals),
            lane_binds: HashMap::new(),
            scratch_view: upload.scratch_view.as_ref().map(|(_, view)| view.clone()),
            dummy_view,
            atlas_view: self.atlas_texture.as_ref().map(|(_, view)| view.clone()),
            first_pass_stamped: false,
            query,
            phases: Vec::new(),
            scissor: patch.map(|(bbox, _)| bbox),
        };
        let target_view = bound
            .texture()
            .create_view(&wgpu::TextureViewDescriptor::default());
        let target_format = bound.texture().format();
        if let Some((_, rects)) = patch {
            // The patched path (ADR 0012): render the frame into the root pair,
            // every pass scissored to the damage bounding box, then replace exactly
            // the damage rectangles on the caller's retained texture.
            executor.realise_masks(&mut recorder)?;
            let root_view = executor.render_plan(&mut recorder, 0)?;
            executor.patch_to_target(
                &mut recorder,
                &root_view,
                &target_view,
                target_format,
                rects,
            );
        } else if matches!(damage, DamagePlan::Patch { .. }) {
            // Every damage rect fell outside the target: nothing visible changed,
            // and honouring the list exactly means touching no pixel at all.
        } else if flat {
            // is_flat checked: a flat root holds drawable ops only.
            let root_ops = compose::run_ops(&encoded.root.ops);
            executor.draw_pass(&mut recorder, &target_view, target_format, true, &root_ops)?;
        } else {
            executor.realise_masks(&mut recorder)?;
            let root_view = executor.render_plan(&mut recorder, 0)?;
            executor.blit_to_target(&mut recorder, &root_view, &target_view, target_format);
        }
        executor.end_stamp(&mut recorder, &target_view);
        let phases = std::mem::take(&mut executor.phases);
        drop(executor);
        if let Some(q) = query {
            recorder.resolve_query_set(&q.set, 0..2, &q.resolve, 0);
            recorder.copy_buffer_to_buffer(&q.resolve, 0, &q.map, 0, 16);
        }
        let execute_wall = compose::submit_and_wait(self, recorder)?;
        Ok((execute_wall, phases))
    }

    /// Decide how this frame treats the viewport's damage list (ADR 0012).
    ///
    /// A `Texture` target retains its contents under the caller's ownership, so a
    /// valid damage list is honoured exactly there. A `Surface` texture's previous
    /// contents are not guaranteed by the swapchain and a `Readback` frame starts
    /// from a fresh texture — neither has anything to patch, so both redraw fully
    /// and say so in a [`Report`].
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // snapped, clamped
    fn plan_damage(
        viewport: &Viewport<'_>,
        into: &Target<'_>,
        reports: &mut Vec<Report>,
    ) -> Result<DamagePlan, RenderError> {
        if viewport.damage.is_empty() {
            return Ok(DamagePlan::Full);
        }
        for (index, rect) in viewport.damage.iter().enumerate() {
            let finite = rect.min.x.is_finite()
                && rect.min.y.is_finite()
                && rect.max.x.is_finite()
                && rect.max.y.is_finite();
            if !finite || rect.min.x > rect.max.x || rect.min.y > rect.max.y {
                return Err(RenderError::InvalidDamage { index });
            }
        }
        let kind = match into {
            Target::Texture(_) => None,
            Target::Surface => Some("Surface"),
            Target::Readback => Some("Readback"),
        };
        if let Some(kind) = kind {
            reports.push(Report {
                kind: ReportKind::DamageNotHonoured,
                detail: format!(
                    "a {kind} target has no retained contents to patch; the full {}x{} \
                     target was redrawn",
                    viewport.width, viewport.height
                ),
            });
            return Ok(DamagePlan::Full);
        }
        // Snap outward to whole pixels, clamp to the target, drop what falls
        // outside entirely.
        let mut rects = Vec::with_capacity(viewport.damage.len());
        let (mut bx0, mut by0, mut bx1, mut by1) = (u32::MAX, u32::MAX, 0_u32, 0_u32);
        for rect in viewport.damage {
            let x0 = rect.min.x.floor().max(0.0) as u32;
            let y0 = rect.min.y.floor().max(0.0) as u32;
            let x1 = (rect.max.x.ceil().max(0.0) as u32).min(viewport.width);
            let y1 = (rect.max.y.ceil().max(0.0) as u32).min(viewport.height);
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            bx0 = bx0.min(x0);
            by0 = by0.min(y0);
            bx1 = bx1.max(x1);
            by1 = by1.max(y1);
            rects.push([x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0)]);
        }
        let bbox = if rects.is_empty() {
            [0, 0, 0, 0]
        } else {
            [bx0, by0, bx1.saturating_sub(bx0), by1.saturating_sub(by0)]
        };
        Ok(DamagePlan::Patch { bbox, rects })
    }

    fn validate_viewport(&self, viewport: &Viewport<'_>) -> Result<(), RenderError> {
        if !viewport.transform.is_finite() {
            return Err(RenderError::NonFiniteViewportTransform);
        }
        let limit = self.limits.max_target_size;
        if viewport.width > limit || viewport.height > limit {
            return Err(RenderError::TargetTooLarge {
                width: viewport.width,
                height: viewport.height,
                limit,
            });
        }
        Ok(())
    }

    /// A zero-size readback is a legitimate frame — a zero-size raster follows from a
    /// zero-size window. The other targets cannot exist at zero size.
    fn zero_size_frame(
        viewport: &Viewport<'_>,
        into: &Target<'_>,
        encoded: &Encoded,
        encode_time: Duration,
        reports: Vec<Report>,
    ) -> Result<Frame, RenderError> {
        match into {
            Target::Readback => Ok(Frame {
                timings: Timings {
                    encode: encode_time,
                    upload: Duration::ZERO,
                    execute: Duration::ZERO,
                    readback: Duration::ZERO,
                    // Nothing executed; a zero wall clock is the honest source.
                    execute_provenance: TimingProvenance::WallClock,
                    phases: Vec::new(),
                },
                counters: Counters {
                    commands: encoded.commands,
                    ..Counters::default()
                },
                reports,
                payload: Payload::Raster(Raster::new(viewport.width, viewport.height, Vec::new())),
            }),
            Target::Surface => Err(RenderError::ZeroSizeTarget { target: "Surface" }),
            Target::Texture(_) => Err(RenderError::ZeroSizeTarget { target: "Texture" }),
        }
    }

    /// Phase 2: create the frame's buffers and textures, sized by phase 1's counts,
    /// and schedule their uploads.
    fn upload(&mut self, encoded: &mut Encoded, viewport: &Viewport<'_>) -> Upload {
        let started = Instant::now();
        let globals = self.gpu.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quorra globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        {
            // Exact for any plausible target size: f32 represents integers up to 2^24.
            #[allow(clippy::cast_precision_loss)]
            let values = [
                viewport.width as f32,
                viewport.height as f32,
                0.0_f32,
                0.0_f32,
            ];
            let mut bytes = [0_u8; 16];
            for (slot, value) in bytes.chunks_exact_mut(4).zip(values) {
                slot.copy_from_slice(&value.to_le_bytes());
            }
            self.queue.write_buffer(&globals, 0, &bytes);
        }
        let mut bytes = 16_u64;
        let make_instances = |gpu: &wgpu::Device, queue: &wgpu::Queue, label, data: &[u8]| {
            if data.is_empty() {
                None
            } else {
                let buffer = gpu.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: data.len() as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&buffer, 0, data);
                Some(buffer)
            }
        };
        let rect_instances = make_instances(
            &self.gpu,
            &self.queue,
            "quorra rect instances",
            &encoded.rect_instances,
        );
        let quad_instances = make_instances(
            &self.gpu,
            &self.queue,
            "quorra quad instances",
            &encoded.quad_instances,
        );
        bytes = bytes
            .saturating_add(encoded.rect_instances.len() as u64)
            .saturating_add(encoded.quad_instances.len() as u64);

        let scratch_view = self.upload_scratch(encoded, &mut bytes);
        self.flush_atlas_tiles(&mut bytes);

        Upload {
            globals,
            rect_instances,
            quad_instances,
            scratch_view,
            bytes,
            time: started.elapsed(),
        }
    }

    /// The frame's scratch coverage image, uploaded whole.
    fn upload_scratch(
        &mut self,
        encoded: &mut Encoded,
        bytes: &mut u64,
    ) -> Option<(wgpu::Texture, wgpu::TextureView)> {
        encoded.scratch.take().map(|scratch| {
            let texture = self.gpu.create_texture(&wgpu::TextureDescriptor {
                label: Some("quorra scratch coverage"),
                size: wgpu::Extent3d {
                    width: scratch.width,
                    height: scratch.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &scratch.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(scratch.width),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: scratch.width,
                    height: scratch.height,
                    depth_or_array_layers: 1,
                },
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            *bytes = bytes.saturating_add(scratch.data.len() as u64);
            (texture, view)
        })
    }

    /// New glyph tiles into the persistent atlas texture (created on first need —
    /// the startup path never pays for it, §7).
    fn flush_atlas_tiles(&mut self, bytes: &mut u64) {
        let pending = self.atlas.take_pending();
        if pending.is_empty() {
            return;
        }
        let (atlas_w, atlas_h) = self.atlas.dimensions();
        let (texture, _) = self.atlas_texture.get_or_insert_with(|| {
            let texture = self.gpu.create_texture(&wgpu::TextureDescriptor {
                label: Some("quorra glyph atlas"),
                size: wgpu::Extent3d {
                    width: atlas_w,
                    height: atlas_h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            (texture, view)
        });
        for tile in pending {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: tile.x,
                        y: tile.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &tile.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(tile.width),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: tile.width,
                    height: tile.height,
                    depth_or_array_layers: 1,
                },
            );
            *bytes =
                bytes.saturating_add(u64::from(tile.width).saturating_mul(u64::from(tile.height)));
        }
    }

    /// Bind the frame's target, validating a caller texture against its contract.
    fn bind_target<'a>(
        &mut self,
        into: &Target<'a>,
        viewport: &Viewport<'_>,
    ) -> Result<Bound<'a>, RenderError> {
        match into {
            Target::Readback => Ok(Bound::Owned(self.gpu.create_texture(
                &wgpu::TextureDescriptor {
                    label: Some("quorra readback target"),
                    size: wgpu::Extent3d {
                        width: viewport.width,
                        height: viewport.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: WARM_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                },
            ))),
            Target::Texture(texture) => {
                Self::validate_texture(texture, viewport)?;
                Ok(Bound::Borrowed(texture))
            }
            Target::Surface => {
                let Some(state) = self.surface.as_mut() else {
                    return Err(RenderError::NoSurface);
                };
                Ok(Bound::Acquired(state.acquire(
                    &self.gpu,
                    viewport.width,
                    viewport.height,
                )?))
            }
        }
    }

    /// The `Target::Texture` contract, checked before anything draws.
    fn validate_texture(
        texture: &wgpu::Texture,
        viewport: &Viewport<'_>,
    ) -> Result<(), RenderError> {
        if texture.format() != WARM_FORMAT {
            return Err(RenderError::TextureFormat {
                got: texture.format(),
            });
        }
        if texture.width() != viewport.width || texture.height() != viewport.height {
            return Err(RenderError::TextureSize {
                got_width: texture.width(),
                got_height: texture.height(),
                need_width: viewport.width,
                need_height: viewport.height,
            });
        }
        if !texture
            .usage()
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            return Err(RenderError::TextureUsage);
        }
        if texture.dimension() != wgpu::TextureDimension::D2
            || texture.sample_count() != 1
            || texture.depth_or_array_layers() != 1
        {
            return Err(RenderError::TextureShape);
        }
        Ok(())
    }

    fn bind_globals(&self, globals: &wgpu::Buffer) -> wgpu::BindGroup {
        let layout = self.pipelines.globals_layout();
        self.gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra globals"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        })
    }

    /// The query set and buffers for one frame's timestamps, when the adapter has
    /// them.
    fn make_pass_query(&self) -> Option<PassQuery> {
        self.timestamps.map(|_| PassQuery::new(&self.gpu))
    }

    /// A frame-internal texture: layer, mask, or ping-pong scratch.
    pub(crate) fn create_internal_texture(
        &self,
        label: &str,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> wgpu::Texture {
        self.gpu.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    }

    pub(crate) fn pipelines(&self) -> &PipelineStore {
        &self.pipelines
    }

    pub(crate) fn gpu(&self) -> &wgpu::Device {
        &self.gpu
    }

    pub(crate) fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// The lane bind group: atlas, scratch, soft mask (dummies where absent).
    pub(crate) fn lane_bind(
        &self,
        atlas: &wgpu::TextureView,
        scratch: &wgpu::TextureView,
        mask: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let layout = self.pipelines.textures_layout();
        self.gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra lane sources"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(atlas),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(scratch),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(mask),
                },
            ],
        })
    }

    /// The composite pass's uniform + bind group for one `ChildOp` (§11.4.5).
    // Offsets below are literal layout positions inside fixed 64/288-byte arrays;
    // the index arithmetic cannot leave them.
    #[allow(clippy::arithmetic_side_effects)]
    pub(crate) fn composite_bind(
        &self,
        op: &ChildOp,
        backdrop: &wgpu::TextureView,
        src: &wgpu::TextureView,
        mask: &wgpu::TextureView,
        scratch: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let mut bytes = [0_u8; 64];
        bytes[0..4].copy_from_slice(&op.mode.to_le_bytes());
        bytes[4..8].copy_from_slice(&op.alpha.to_le_bytes());
        for (i, v) in op.clip_rect.iter().enumerate() {
            let at = 16 + i * 4;
            bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        bytes[40..44].copy_from_slice(&op.residue_origin[0].to_le_bytes());
        bytes[44..48].copy_from_slice(&op.residue_origin[1].to_le_bytes());
        for (i, v) in op.residue_rect.iter().enumerate() {
            let at = 48 + i * 4;
            bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        let uniform = self.gpu.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quorra composite params"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&uniform, 0, &bytes);
        let layout = self.pipelines.composite_layout();
        self.gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra composite"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(backdrop),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(mask),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(scratch),
                },
            ],
        })
    }

    /// The reduce pass's uniform + bind group for one mask (§11.5; byte-agreed).
    #[allow(clippy::arithmetic_side_effects)] // fixed-layout offsets in a 288-byte array
    pub(crate) fn reduce_bind(&self, plan: &MaskPlan, src: &wgpu::TextureView) -> wgpu::BindGroup {
        let mut bytes = [0_u8; 288];
        bytes[0..4].copy_from_slice(&plan.kind_word.to_le_bytes());
        for (i, v) in plan.backdrop.iter().enumerate() {
            let at = 16 + i * 4;
            bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        bytes[32..288].copy_from_slice(&plan.table);
        let uniform = self.gpu.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quorra reduce params"),
            size: 288,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&uniform, 0, &bytes);
        let layout = self.pipelines.reduce_layout();
        self.gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra reduce"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(src),
                },
            ],
        })
    }

    /// The blit pass's bind group.
    pub(crate) fn blit_bind(&self, src: &wgpu::TextureView) -> wgpu::BindGroup {
        let layout = self.pipelines.blit_layout();
        self.gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra blit"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(src),
            }],
        })
    }

    /// Realise the frame's referenced images, ramps and meshes as textures, once
    /// per resident resource — created here rather than at upload so startup and
    /// pages without them never pay (§7). Returns the bytes written.
    ///
    /// The ids were validated during encode; a miss here still refuses by name
    /// rather than trusting that invariant silently.
    fn ensure_paint_textures(&mut self, encoded: &Encoded) -> Result<u64, RenderError> {
        let mut bytes = 0_u64;
        for &id in &encoded.used_images {
            if self.image_textures.contains_key(&id) {
                continue;
            }
            let Some(stored) = self.resources.image(ImageId(id)) else {
                return Err(RenderError::UnknownImage { image: ImageId(id) });
            };
            let spec = stored.spec.clone();
            let pair = self.rgba_texture("quorra image", spec.width, spec.height, &spec.data);
            bytes = bytes.saturating_add(spec.data.len() as u64);
            self.image_textures.insert(id, pair);
        }
        for &id in &encoded.used_ramps {
            if self.ramp_textures.contains_key(&id) {
                continue;
            }
            let Some(stored) = self.resources.ramp(RampId(id)) else {
                return Err(RenderError::UnknownRamp { ramp: RampId(id) });
            };
            let samples = sample_ramp(&stored.stops);
            let pair = self.rgba_texture("quorra ramp", 256, 1, &samples);
            bytes = bytes.saturating_add(samples.len() as u64);
            self.ramp_textures.insert(id, pair);
        }
        for &id in &encoded.used_meshes {
            if self.mesh_textures.contains_key(&id) {
                continue;
            }
            let Some(stored) = self.resources.mesh(MeshId(id)) else {
                return Err(RenderError::UnknownMesh { mesh: MeshId(id) });
            };
            let spec = stored.spec.image.clone();
            let pair = self.rgba_texture("quorra mesh", spec.width, spec.height, &spec.data);
            bytes = bytes.saturating_add(spec.data.len() as u64);
            self.mesh_textures.insert(id, pair);
        }
        Ok(bytes)
    }

    /// One straight-alpha RGBA8 texture, uploaded whole.
    fn rgba_texture(
        &self,
        label: &str,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = self.gpu.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width.saturating_mul(4)),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    /// The image quad's uniform + bind group for one `ImageOp` (ISO 32000-2
    /// §8.9.5; layout mirrored in `image.wgsl`'s `Params`).
    #[allow(clippy::arithmetic_side_effects)] // fixed-layout offsets in a 112-byte array
    #[allow(clippy::cast_precision_loss)] // target sizes are far below 2^24
    pub(crate) fn image_bind(
        &self,
        op: &ImageOp,
        width: u32,
        height: u32,
        mask: &wgpu::TextureView,
        scratch: &wgpu::TextureView,
    ) -> Result<wgpu::BindGroup, RenderError> {
        let Some((_, image_view)) = self.image_textures.get(&op.image) else {
            return Err(RenderError::UnknownImage {
                image: ImageId(op.image),
            });
        };
        let mut bytes = [0_u8; 112];
        let mut put = |at: usize, v: f32| bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
        for (i, v) in op.inv.iter().enumerate() {
            put(i * 4, *v); // inv0 then inv1.xy
        }
        put(24, op.alpha);
        put(28, if op.linear { 1.0 } else { 0.0 });
        for (i, v) in op.image_rect.iter().enumerate() {
            put(32 + i * 4, *v);
        }
        for (i, v) in op.dest.iter().enumerate() {
            put(48 + i * 4, *v);
        }
        for (i, v) in op.clip.iter().enumerate() {
            put(64 + i * 4, *v);
        }
        let origin = op.residue_origin.unwrap_or([0.0, 0.0]);
        put(80, origin[0]);
        put(84, origin[1]);
        put(
            88,
            if op.residue_origin.is_some() {
                1.0
            } else {
                0.0
            },
        );
        put(92, if op.axis_aligned { 1.0 } else { 0.0 });
        put(96, width as f32);
        put(100, height as f32);
        let uniform = self.quad_uniform("quorra image params", &bytes);
        let layout = self.pipelines.image_layout();
        Ok(self.gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra image"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(image_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(mask),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(scratch),
                },
            ],
        }))
    }

    /// The shading quad's uniform + bind group for one `ShadedOp` (ISO 32000-2
    /// §8.7.4.5; layout mirrored in `shading.wgsl`'s `Params`).
    #[allow(clippy::arithmetic_side_effects)] // fixed-layout offsets in a 144-byte array
    #[allow(clippy::cast_precision_loss)] // extend bits ≤ 3; sizes far below 2^24
    pub(crate) fn shaded_bind(
        &self,
        op: &ShadedOp,
        width: u32,
        height: u32,
        scratch: &wgpu::TextureView,
        mask: &wgpu::TextureView,
    ) -> Result<wgpu::BindGroup, RenderError> {
        let paint_view = match op.paint {
            PaintSource::Ramp(id) => {
                let Some((_, view)) = self.ramp_textures.get(&id) else {
                    return Err(RenderError::UnknownRamp { ramp: RampId(id) });
                };
                view
            }
            PaintSource::Mesh(id) => {
                let Some((_, view)) = self.mesh_textures.get(&id) else {
                    return Err(RenderError::UnknownMesh { mesh: MeshId(id) });
                };
                view
            }
        };
        let mut bytes = [0_u8; 144];
        let mut put = |at: usize, v: f32| bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
        for (i, v) in op.inv.iter().enumerate() {
            put(i * 4, *v); // inv0 then inv1.xy
        }
        put(24, op.kind_word);
        put(28, op.extend_bits as f32);
        for (i, v) in op.geo0.iter().enumerate() {
            put(32 + i * 4, *v);
        }
        for (i, v) in op.geo1.iter().enumerate() {
            put(48 + i * 4, *v);
        }
        for (i, v) in op.dest.iter().enumerate() {
            put(64 + i * 4, *v);
        }
        let origin = op.coverage_origin.unwrap_or([0.0, 0.0]);
        put(80, origin[0]);
        put(84, origin[1]);
        put(
            88,
            if op.coverage_origin.is_some() {
                1.0
            } else {
                0.0
            },
        );
        for (i, v) in op.coverage_rect.iter().enumerate() {
            put(96 + i * 4, *v);
        }
        for (i, v) in op.clip.iter().enumerate() {
            put(112 + i * 4, *v);
        }
        put(128, width as f32);
        put(132, height as f32);
        let uniform = self.quad_uniform("quorra shading params", &bytes);
        let layout = self.pipelines.shading_layout();
        Ok(self.gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra shading"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(paint_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(scratch),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(mask),
                },
            ],
        }))
    }

    /// One single-quad uniform buffer, written whole.
    fn quad_uniform(&self, label: &str, bytes: &[u8]) -> wgpu::Buffer {
        let uniform = self.gpu.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&uniform, 0, bytes);
        uniform
    }

    /// The 1×1 stand-in for absent coverage sources and masks: **white**, so an
    /// absent soft mask admits everything.
    fn ensure_dummy(&mut self) -> wgpu::TextureView {
        if self.dummy_texture.is_none() {
            let texture = self.gpu.create_texture(&wgpu::TextureDescriptor {
                label: Some("quorra dummy white"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &[255],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(1),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            self.dummy_texture = Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
        }
        // Just created above when absent.
        #[allow(clippy::expect_used)]
        self.dummy_texture.clone().expect("created above")
    }
}

/// Sample a validated ramp to 256 straight-RGBA8 texels, on the CPU (ADR 0011).
///
/// The shader indexes the result with `textureLoad` at `round(t·255)`, so the
/// sweep's colour arithmetic is this function's — deterministic across adapters —
/// rather than the driver's texture filtering.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // round of 0..=255
#[allow(clippy::cast_precision_loss)] // i ≤ 255
fn sample_ramp(stops: &[Stop]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1024);
    for i in 0..256_u32 {
        let color = ramp_color_at(stops, i as f32 / 255.0);
        for component in [color.r, color.g, color.b, color.a] {
            // Components were validated into 0..=1 at upload.
            out.push((component * 255.0).round() as u8);
        }
    }
    out
}

/// The ramp's colour at `t`: constant before the first and after the last stop,
/// linearly interpolated between neighbours. At coincident offsets the later stop
/// wins — a PDF type 2/3 stitching boundary is half-open, the next function owning
/// its start (ISO 32000-2 §7.10.4).
fn ramp_color_at(stops: &[Stop], t: f32) -> Color {
    // Upload refused empty ramps; transparent black would still be an honest
    // answer for one, not an approximation of anything.
    let Some(first) = stops.first() else {
        return Color::new(0.0, 0.0, 0.0, 0.0);
    };
    if t <= first.offset {
        return first.color;
    }
    let mut previous = *first;
    for stop in stops.iter().skip(1) {
        if t <= stop.offset {
            let span = stop.offset - previous.offset;
            if span <= 0.0 {
                return stop.color;
            }
            let u = (t - previous.offset) / span;
            let mix = |a: f32, b: f32| a + (b - a) * u;
            return Color::new(
                mix(previous.color.r, stop.color.r),
                mix(previous.color.g, stop.color.g),
                mix(previous.color.b, stop.color.b),
                mix(previous.color.a, stop.color.a),
            );
        }
        previous = *stop;
    }
    previous.color
}
