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
//!
//! # This file, and the modules under it
//!
//! Each part is named for its one thing, and each is private: a caller writes
//! `quorra_gpu::device::Device`, and the layout below is ours to change (ADR 0051).
//! The names below are not links for the same reason — a private module is not in the
//! published documentation, so this list is the only place the structure survives into
//! it.
//!
//! - `ramp` — a colour ramp sampled to texels, which is arithmetic rather than a
//!   device (ADR 0011).
//! - `binds` — the bind group and the uniform bytes each of the compositor's passes
//!   reads.
//! - `rare` — the same for the image and shading quads, which the brief's §0 calls the
//!   rare case.
//! - `textures` — the textures a device makes, and the usages each one asks for.

mod binds;
mod ramp;
mod rare;
mod textures;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use quorra_scene::Scene;

use quorra_scene::{
    FnOp, FunctionId, ImageId, ImageSpec, MeshId, MeshSpec, OutlineId, RampId, ResourceId, Segment,
    Stop,
};

use self::ramp::{RAMP_RESOLUTION, sample_ramp};

use crate::atlas::AtlasStore;
use crate::compose::{self, Executor, PassLoad, Region};
use crate::encode::{self, Encoded};
use crate::error::{DeviceError, RenderError};
use crate::frame::{Counters, EncodeSource, Frame, Payload, Raster, TimingProvenance, Timings};
use crate::layers::{self, LayerPool};
use crate::pipeline::{PipelineStore, WARM_FORMAT};
use crate::readback;
use crate::report::{Report, ReportKind};
use crate::resources::ResourceStore;
use crate::retained::{EncodeKey, RetainedScene};
use crate::startup::{self, Coverage, Options, PreSteps, StartupTimings, WarmUp};
use crate::surface::SurfaceState;
use crate::target::Target;
pub(crate) use crate::timing::PassQuery;
use crate::timing::{self, TimestampSupport};
use crate::viewport::Viewport;

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

/// Hands out device numbers. Monotonic for the process, which is all a
/// [`RetainedScene`] needs of it: a handle carried to a device other than the one that
/// encoded it must not replay, and two live devices never share a number (ADR 0048).
static NEXT_DEVICE_ID: AtomicU64 = AtomicU64::new(0);

/// The rendering device: an adapter, a queue, and the pipelines a scene needs.
///
/// Constructible on a background thread and not requiring one (§2.1). Headless is the
/// first-class form — it is what the caller's test suite and correctness oracle use.
#[derive(Debug)]
pub struct Device {
    /// This device's number among the devices this process has made, so that a retained
    /// encode — which names atlas positions and resource ids belonging to one device —
    /// cannot be replayed through another.
    id: u64,
    gpu: wgpu::Device,
    queue: wgpu::Queue,
    description: String,
    limits: Limits,
    pipelines: Arc<PipelineStore>,
    /// The warm-up thread, held so that [`Drop`] can join it: a thread compiling
    /// inside the driver may not outlive the device it compiles for (ADR 0018).
    /// `None` when the host could not give us a thread and the warm set compiled
    /// inline.
    warm_up: Option<thread::JoinHandle<()>>,
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
    /// Which lane makes coverage bytes, and how finely the GPU one samples (ADR 0016).
    coverage: Coverage,
    /// Whether a frame subdivides its encode phase (ADR 0023).
    instrument_encode: bool,
    coverage_samples: u32,
    /// The GPU lane's winding target, kept across frames (ADR 0016's measurement).
    winding_texture: crate::winding::WindingTexture,
    /// A layer texture made ahead of the frame that will want it (ADR 0035), and held
    /// only until that frame takes it — the pool itself stays per-frame, as ADR 0012
    /// decided. Worth 0.06 ms and no more; ADR 0040 measured it and says why it stays.
    warmed_layer: Option<(u32, u32, wgpu::Texture)>,
    timestamps: Option<TimestampSupport>,
    /// The frame's two timestamps and their buffers, kept for the device's life
    /// (ADR 0031). Absent when the adapter has no timestamp queries, and absent for
    /// one frame after a read fails, which is what returns a poisoned map buffer to a
    /// fresh one.
    pass_query: Option<PassQuery>,
    surface: Option<SurfaceState>,
    /// The blocking startup steps, each measured on its own (§7, and the caller's
    /// feedback §8.1: one number that measured three could not be attributed).
    startup: StartupSteps,
}

/// What the four blocking steps of construction cost, in the order they happen.
#[derive(Debug, Clone, Copy)]
struct StartupSteps {
    /// `None` when the instance was the caller's, not ours to time.
    instance_creation: Option<Duration>,
    surface_creation: Duration,
    adapter_selection: Duration,
    device_creation: Duration,
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
        let instance = startup::create_instance();
        let pre = PreSteps {
            instance_creation: Some(started.elapsed()),
            surface_creation: Duration::ZERO,
        };
        Self::build(&instance, None, pre, options)
    }

    /// A headless device on an instance the caller already has.
    ///
    /// The hoisting entry point of [`startup::create_instance`] without a window:
    /// useful when a host makes one instance early and builds both its offscreen and
    /// its windowed devices from it. [`StartupTimings::instance_creation`] is `None`
    /// on the result, because that step was not this constructor's.
    ///
    /// # Errors
    ///
    /// As [`Device::headless`].
    pub fn headless_with_instance(
        instance: &wgpu::Instance,
        options: &Options,
    ) -> Result<Self, DeviceError> {
        Self::build(instance, None, PreSteps::BORROWED_INSTANCE, options)
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
        let instance = startup::create_instance();
        let instance_creation = started.elapsed();
        Self::on_surface(&instance, Some(instance_creation), window, options)
    }

    /// A device that presents to a window, on an instance the caller made earlier.
    ///
    /// **The startup lever**: an instance needs no window, no surface and no event
    /// loop, so [`startup::create_instance`] can run on a thread started at `main`'s
    /// first line while the document is read and the window opened, and this
    /// constructor takes the result. The caller measured about 20 ms of a 145 ms
    /// launch in that overlap (their feedback §8.2). What cannot be hoisted, and is
    /// not claimed: `request_adapter` takes the surface as `compatible_surface`, so
    /// adapter selection is genuinely downstream of the window.
    ///
    /// [`StartupTimings::instance_creation`] is `None` on the result — the host that
    /// made the instance is the one that can time it.
    ///
    /// # Errors
    ///
    /// As [`Device::for_surface`].
    pub fn for_surface_with_instance(
        instance: &wgpu::Instance,
        window: impl Into<wgpu::SurfaceTarget<'static>>,
        options: &Options,
    ) -> Result<Self, DeviceError> {
        Self::on_surface(instance, None, window, options)
    }

    /// The shared body of the two surface constructors: create the surface, then
    /// build, keeping each step's cost separate.
    fn on_surface(
        instance: &wgpu::Instance,
        instance_creation: Option<Duration>,
        window: impl Into<wgpu::SurfaceTarget<'static>>,
        options: &Options,
    ) -> Result<Self, DeviceError> {
        let started = Instant::now();
        let surface = instance
            .create_surface(window)
            .map_err(|source| DeviceError::SurfaceCreation { source })?;
        let pre = PreSteps {
            instance_creation,
            surface_creation: started.elapsed(),
        };
        Self::build(instance, Some(surface), pre, options)
    }

    /// The names of every adapter wgpu can see on this machine, for choosing an
    /// [`Options::adapter`] filter — and for the cross-adapter byte-equality gate,
    /// which renders on all of them (§4.6, §11.4).
    ///
    /// Every backend, because the instance is this function's own. A host that
    /// restricted the backend set ([`startup::create_instance_with`]) must ask
    /// [`Device::adapter_names_on`] instead, or it will offer a choice its own
    /// constructors cannot honour.
    #[must_use]
    pub fn adapter_names() -> Vec<String> {
        let instance = startup::create_instance();
        Self::adapter_names_on(&instance)
    }

    /// The names of every adapter *this instance* can see: the same list
    /// [`Device::adapter_names`] returns, narrowed to the backends the instance was
    /// built with.
    ///
    /// Which is why it takes the instance rather than a backend set — it answers with
    /// what the constructors given the same instance will actually choose among, and
    /// nothing else can promise that. One GPU appears once per backend that can drive
    /// it, under the same device name each time, so a duplicated name in the result of
    /// [`Device::adapter_names`] is that and not a second card.
    #[must_use]
    pub fn adapter_names_on(instance: &wgpu::Instance) -> Vec<String> {
        pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
            .iter()
            .map(|adapter| adapter.get_info().name)
            .collect()
    }

    fn build(
        instance: &wgpu::Instance,
        surface: Option<wgpu::Surface<'static>>,
        pre: PreSteps,
        options: &Options,
    ) -> Result<Self, DeviceError> {
        let adapter_started = Instant::now();
        let adapter = startup::select_adapter(instance, surface.as_ref(), options)?;
        let adapter_selection = adapter_started.elapsed();

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
        let device_creation = device_started.elapsed();

        let timestamps = required_features
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then(|| TimestampSupport {
                period: queue.get_timestamp_period(),
            });

        let pipelines = PipelineStore::new(gpu.clone());
        // A surface device warms the presenting lanes in the surface's own format too,
        // so its first frame does not compile them inside itself (ADR 0043).
        let warm_up = pipelines.spawn_warm_up(surface_state.as_ref().map(SurfaceState::format));

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

        // Before the device is assembled, because the constructor owns `gpu` until then
        // — and this is the point of making it here at all (ADR 0031).
        let pass_query_at_startup = timestamps.map(|_| PassQuery::new(&gpu));

        let max_dimension = gpu.limits().max_texture_dimension_2d;
        let limits = Limits {
            max_target_size: max_dimension,
            max_frame_bytes: options.max_frame_bytes,
            max_resource_bytes: options.max_resource_bytes,
        };

        Ok(Self {
            id: NEXT_DEVICE_ID.fetch_add(1, Ordering::Relaxed),
            gpu,
            queue,
            description,
            limits,
            pipelines,
            warm_up,
            resources: ResourceStore::new(options.max_resource_bytes),
            atlas: AtlasStore::new(options.atlas_budget, max_dimension),
            atlas_texture: None,
            image_textures: HashMap::new(),
            ramp_textures: HashMap::new(),
            mesh_textures: HashMap::new(),
            linear_sampler,
            dummy_texture: None,
            glyph_quantum: options.glyph_quantum,
            coverage: options.coverage,
            instrument_encode: options.instrument_encode,
            winding_texture: crate::winding::WindingTexture::default(),
            warmed_layer: None,
            // Rounded to a square grid and bounded, here rather than at the call site:
            // an option is a request, and what the lane can actually sample is ours.
            coverage_samples: {
                let side = options.coverage_samples.clamp(4, 64).isqrt().max(2);
                side.saturating_mul(side)
            },
            timestamps,
            // Made here rather than on the frame that first wants it: the driver charges
            // 2.43 ms for a `QuerySet` and its two buffers the first time, and a device is
            // constructed off the critical path by every host that follows §7's advice
            // — where a first frame is on it by definition (ADR 0031).
            pass_query: pass_query_at_startup,
            surface: surface_state,
            startup: StartupSteps {
                instance_creation: pre.instance_creation,
                surface_creation: pre.surface_creation,
                adapter_selection,
                device_creation,
            },
        })
    }

    /// The pipeline cache, for the passes that live in modules of their own.
    #[allow(dead_code)] // `winding.rs`'s own tests build a device to reach its pipelines
    pub(crate) fn pipeline_store(&self) -> &PipelineStore {
        &self.pipelines
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

    /// Which lane makes coverage bytes for the frames after this call.
    #[must_use]
    pub fn coverage(&self) -> Coverage {
        self.coverage
    }

    /// Choose the coverage lane for the frames after this call.
    ///
    /// **Per frame, because the right answer changes within a session.** The two lanes
    /// have opposite cost curves and the crossover is a magnification (ADR 0016): the
    /// CPU lane's atlas wins while glyphs are small and repeated, and the GPU lane wins
    /// once a glyph costs more to fill than to describe. Only a caller knows which side
    /// of that its next frame is on, and a choice fixed at construction would make it
    /// choose once for a session in which a person zooms.
    ///
    /// Switching costs nothing but the cache that goes idle: the glyph atlas and the
    /// winding texture both live on the device and survive frames drawn by the other
    /// lane, so alternating does not throw either away. What it *does* change is the
    /// bytes: coverage from the two lanes differs on antialiased edges within the bound
    /// ADR 0016 states, so a caller comparing frames across a switch is comparing two
    /// answers to the same question rather than one answer twice.
    pub fn set_coverage(&mut self, coverage: Coverage) {
        self.coverage = coverage;
    }

    /// Upload an outline: validated, priced against the resource budget, resident
    /// until [`Device::release`]. The id is what a scene's `fill`/`stroke`/`clip`
    /// reference — uploaded once, referenced many times (§2.2 of the brief: the
    /// caller keys these by `Arc::as_ptr` identity, so a zoom re-uploads nothing).
    ///
    /// # Errors
    ///
    /// [`DeviceError::InvalidResource`] naming what §4.7 refused, or
    /// [`DeviceError::ResourceBudgetExceeded`] naming all three numbers. A device that
    /// has issued all `u32::MAX` identifiers refuses with
    /// [`DeviceError::ResourceIdsExhausted`] — ids are never reused, because a reissued
    /// one would make a retained encode draw a resource it did not name (ADR 0050).
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

    /// Upload a §7.10.5 type 4 function for [`Paint::Function`] to name (ADR 0053).
    ///
    /// **This is where a program is refused, and that placement is the whole point.** A
    /// program that cannot be lowered to a shader — a backward jump, a `roll` whose count
    /// came off the stack, a transcendental whose value reaches a comparison — is refused
    /// *here*, before the caller has built a scene, so that its fallback costs a branch
    /// rather than a page. `Device::render` never refuses a paint for anything about the
    /// program itself.
    ///
    /// One upload serves any number of shadings: the domain, matrix, range and background
    /// live on the paint, so two placements of one program share this identifier and the
    /// one generated shader it is cached by. What a page pays per *distinct* program is
    /// one shader compile, which is what `Scene::cost`'s `function_programs` counts.
    ///
    /// # Errors
    ///
    /// [`DeviceError::InvalidFunction`] naming which of the upload's three questions was
    /// answered no — the structure, the analysing walk, or ADR 0053 §3's agreement
    /// classification — and by what. Also [`DeviceError::ResourceBudgetExceeded`] and
    /// [`DeviceError::ResourceIdsExhausted`], as [`Device::upload_outline`].
    ///
    /// [`Paint::Function`]: quorra_scene::Paint::Function
    pub fn upload_function(&mut self, program: &[FnOp]) -> Result<FunctionId, DeviceError> {
        self.resources.upload_function(program)
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
        // Read before the release, because the analysis that carries the hash is exactly
        // what the release removes.
        let released_program = match id {
            ResourceId::Function(program) => self
                .resources
                .function(program)
                .map(|stored| stored.analysis.program_hash()),
            _ => None,
        };
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
            // A released program takes its compiled pipelines with it, unless another
            // resident program has the same instructions and would name the same key:
            // they are keyed by content, so keeping them would hold GPU memory for
            // something nothing can ask for again, and dropping one that is still
            // reachable would cost a recompile rather than a wrong picture.
            ResourceId::Function(_) => {
                if let Some(hash) = released_program
                    && !self.resources.holds_program(hash)
                {
                    self.pipelines.forget_program(hash);
                }
            }
            // An outline has no device-resident twin.
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
    ///
    /// **A host that polls this must poll [`Device::warm_up`] instead**, or handle the
    /// two outcomes in which `false` is the final answer: a warm-up whose pipelines
    /// this adapter refused, and one that ended in a panic. `is_warm` is that question
    /// narrowed to its one interesting answer, and a loop waiting for it to turn true
    /// is a loop that can never end.
    #[must_use]
    pub fn is_warm(&self) -> bool {
        matches!(self.pipelines.warm_up(), WarmUp::Warm(_))
    }

    /// Where the background warm-up has got to, without blocking: still running, warm,
    /// refused by name, or ended without an answer.
    ///
    /// The whole reason this exists beside [`Device::is_warm`] is §5's rule applied to
    /// startup — a caller that waits for the warm set has to be able to learn that it
    /// is never coming, and a boolean cannot say that.
    #[must_use]
    pub fn warm_up(&self) -> WarmUp {
        self.pipelines.warm_up()
    }

    /// Make the frame-sized resources a target of this size will need, now (ADR 0035).
    ///
    /// **What this is worth, measured** (ADR 0040): one texture of that size, whose
    /// creation costs **0.04 to 0.06 ms** cold on RADV and a tenth of that warm. It is
    /// not the fourteen milliseconds ADR 0035 recorded — that number could not be
    /// reproduced in any of five configurations, including the one where the texture is
    /// claimed, and the mechanism cannot buy more than the allocation it moves.
    ///
    /// **What a first frame actually pays over its successors is 1.5 to 6 ms, and it does
    /// not scale with the target**: a page with groups pays the same excess at
    /// 1 191 × 1 684 and at 2 448 × 4 752, and a throwaway warm frame at 64 × 64 removes
    /// three quarters of what one at the target's own size removes. On a page with a
    /// group, 0.75 to 2.6 ms of it was two first-use pipeline compilations, and those are
    /// compiled at construction now, on the thread nothing blocks on. What is left is the
    /// driver's first submission, which no size hint reaches.
    ///
    /// So call it if you like — it costs a caller nothing and it is what a driver that
    /// commits memory at allocation rather than at first use would want — but do not
    /// budget a first frame around it. Call it where the device is constructed: §7's
    /// advice already puts that off the critical path (the caller's `main` spawns a
    /// thread for it at its first line), while a first frame is on that path by
    /// definition. Calling it again with the same size is free; with a different one it
    /// replaces what it held, because a viewer draws one size at a time and a zoom
    /// replaces it.
    ///
    /// It is a hint and nothing depends on it: a frame of any size draws correctly
    /// whether or not this was called, and what a `Frame` reports about its own bytes is
    /// what the frame *needed*, not what happened to be resident already.
    pub fn warm_for(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        // Held until the frame that wants it takes it, and no longer: the pool itself
        // stays per-frame, because ADR 0012 declined to keep one and nothing measured
        // here overturns that.
        //
        // One texture, since ADR 0038: a plan accumulates in one rather than ping-ponging
        // between two. Since ADR 0039 the root is as big as what the page marks, so this
        // is the size the root asks for on the 26 % of layered frames that mark their
        // whole target and is dropped unused on the rest — which is a cost of 0.06 ms,
        // and why ADR 0040 left the pool's exact-extent matching alone.
        self.warmed_layer = Some((width, height, layers::warm_texture(self, width, height)));
    }

    /// Block until the warm-up has finished. Startup measurement support; a caller
    /// that does not care never needs to call it.
    ///
    /// Returns whatever became of the warm-up, not only success: a set this adapter
    /// refused and a thread that panicked both end the wait, and [`Device::warm_up`]
    /// then says which. The one thing this never does is not return (ADR 0042).
    pub fn wait_until_warm(&self) {
        drop(self.pipelines.wait_until_warm());
    }

    /// What startup cost, one number per step that can regress on its own (§7).
    /// `pipeline_compilation` is `None` until the background warm-up finishes, and
    /// `instance_creation` is `None` when the instance was the caller's.
    #[must_use]
    pub fn startup(&self) -> StartupTimings {
        StartupTimings {
            instance_creation: self.startup.instance_creation,
            surface_creation: self.startup.surface_creation,
            adapter_selection: self.startup.adapter_selection,
            device_creation: self.startup.device_creation,
            pipeline_compilation: self.pipelines.warm_duration(),
        }
    }

    /// Force the surface to be reconfigured — a fresh swapchain — before the next
    /// [`Target::Surface`] frame.
    ///
    /// The host's lever for a presentation stack it suspects is wedged: the surface
    /// itself reports [`SurfaceProblem`](crate::error::SurfaceProblem)s and asks for
    /// its own reconfiguration where it can tell, but a host that knows better —
    /// after a run of refusals, or a compositor event this library cannot see — need
    /// not wait for that or fake a resize. Costs nothing until the next surface
    /// frame, which pays one reconfigure.
    ///
    /// # Errors
    ///
    /// [`RenderError::NoSurface`] on a device constructed with
    /// [`Device::headless`] — asking to invalidate a surface that cannot exist is a
    /// caller bug, and hiding it would hide the defect.
    pub fn invalidate_surface(&mut self) -> Result<(), RenderError> {
        let Some(surface) = self.surface.as_mut() else {
            return Err(RenderError::NoSurface);
        };
        surface.invalidate();
        Ok(())
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
        let encoded = self.encode_scene(scene, viewport)?;
        let encode_time = encode_started.elapsed();

        let mut frame = self.draw_encoded(
            &encoded,
            viewport,
            &into,
            &damage,
            reports,
            encode_time,
            EncodeSource::Encoded,
        )?;
        // Reported on the frame that caused it rather than on the one that pays for it:
        // this is the frame whose atlas layout stopped being the layout, and a caller
        // holding a `RetainedScene` learns here that its encode is now stale.
        frame.counters.atlas_repacked = self.settle_atlas(&encoded);
        Ok(frame)
    }

    /// Render one frame of a scene the caller retains, replaying the encode of its last
    /// frame when nothing an encode depends on has changed (ADR 0048).
    ///
    /// **The same pixels as [`Device::render`] over the same scene and viewport**, and
    /// the same refusals: a replayed frame skips only phase 1, and every check phase 1
    /// is not — the frame budget for internal textures, the target's own contract, the
    /// passes themselves — runs on every call. A frame that would be refused is refused
    /// identically, whether it encoded or replayed, because an encode is retained only
    /// after it succeeded and is discarded the moment any input it read has moved.
    ///
    /// [`Frame::encode_source`] says which of the two this frame was. What survives
    /// which change, and what no design can make survive, is
    /// [`RetainedScene`]'s own table.
    ///
    /// # Errors
    ///
    /// Exactly [`Device::render`]'s, over the scene the handle holds.
    ///
    /// [`Frame::encode_source`]: crate::frame::Frame::encode_source
    // As `render`: a target is a discriminant plus a borrow.
    #[allow(clippy::needless_pass_by_value)]
    pub fn render_retained(
        &mut self,
        retained: &mut RetainedScene,
        viewport: &Viewport<'_>,
        into: Target<'_>,
    ) -> Result<Frame, RenderError> {
        self.validate_viewport(viewport)?;

        let mut reports = Vec::new();
        let damage = Self::plan_damage(viewport, &into, &mut reports)?;

        // The key is taken before the encode, and that is the safe order: encoding
        // inserts atlas tiles, which never move an existing entry, so the generation it
        // reads is the one the encode is valid under — while a *reset* triggered by this
        // frame bumps it afterwards and invalidates what this frame stored, which is
        // exactly right, since a repack moves every tile the instances name.
        let key = EncodeKey::new(
            self.id,
            viewport,
            self.coverage,
            self.atlas.generation,
            self.resources.generation(),
        );
        let encode_started = Instant::now();
        // Borrowed from `retained`, which is not `self`: the encode and the device it
        // draws through are two objects, so nothing here needs a clone.
        let (source, encoded) =
            retained.prepare(key, |scene| self.encode_scene(scene, viewport))?;
        let encode_time = encode_started.elapsed();

        let mut frame = self.draw_encoded(
            encoded,
            viewport,
            &into,
            &damage,
            reports,
            encode_time,
            source,
        );
        // A replay inserted nothing, so there is nothing for a repack to settle — and
        // resetting after one would bump the generation the replayed encode is keyed
        // under and cost the next frame an encode for no reason.
        if let Ok(frame) = frame.as_mut()
            && source == EncodeSource::Encoded
        {
            frame.counters.atlas_repacked = self.settle_atlas(encoded);
        }
        frame
    }

    /// Phase 1: classify, rasterise coverage, and count (`encode.rs`).
    fn encode_scene(
        &mut self,
        scene: &Scene,
        viewport: &Viewport<'_>,
    ) -> Result<Encoded, RenderError> {
        encode::encode(
            scene,
            viewport,
            self.limits.max_frame_bytes,
            self.limits.max_target_size,
            &self.resources,
            &mut self.atlas,
            self.glyph_quantum,
            self.coverage,
            self.instrument_encode,
        )
    }

    /// Phases 2 to 4 of a frame: price, allocate, upload, draw, resolve.
    ///
    /// Everything that is not phase 1, which is the seam a retained encode is replayed
    /// across (ADR 0048): the two callers differ only in where the [`Encoded`] came
    /// from, and every refusal below this line is taken by both.
    #[allow(clippy::too_many_arguments)] // one frame's inputs, named once at two call sites
    fn draw_encoded(
        &mut self,
        encoded: &Encoded,
        viewport: &Viewport<'_>,
        into: &Target<'_>,
        damage: &DamagePlan,
        reports: Vec<Report>,
        encode_time: Duration,
        source: EncodeSource,
    ) -> Result<Frame, RenderError> {
        // Before the zero-size branch and before anything is drawn: what a frame says
        // about itself has to be true of a frame that draws nothing too, and the claim
        // this report makes is about the scene rather than about the pixels.
        let mut reports = reports;
        encode::empty_stack_reports(&encoded.used_functions, &self.resources, &mut reports);
        if viewport.width == 0 || viewport.height == 0 {
            return Self::zero_size_frame(viewport, into, encoded, encode_time, reports, source);
        }

        // Price the compositor's internal textures while nothing of the frame
        // exists yet (§5: count then allocate; the refusal names both numbers).
        // Before the target is bound on purpose: a `Surface` refusal must cost no
        // swapchain acquire, because a texture acquired and then dropped unpresented
        // leaves the swapchain a semaphore no submission will ever wait on — the
        // viewer measured that as every later acquire timing out, permanently.
        // A patched frame renders through the root pair even when flat.
        let patches = matches!(damage, DamagePlan::Patch { rects, .. } if !rects.is_empty());
        let internal_bytes =
            layers::internal_texture_bytes(encoded, viewport.width, viewport.height, patches);
        if internal_bytes > self.limits.max_frame_bytes {
            return Err(RenderError::FrameBudgetExceeded {
                needed: internal_bytes,
                budget: self.limits.max_frame_bytes,
            });
        }

        // Phase 2: allocate (sized by phase 1) and schedule uploads — including
        // the device-resident form of any image, ramp or mesh drawn for the first
        // time this frame.
        let paint_started = Instant::now();
        let paint_bytes = self.ensure_paint_textures(encoded)?;
        let paint_time = paint_started.elapsed();
        let upload = self.upload(encoded)?;
        let upload_time = upload.time.saturating_add(paint_time);
        let upload_bytes = upload.bytes.saturating_add(paint_bytes);

        // Every refusal a scene can earn has been taken; bind the target last, so
        // the acquire happens only for a frame that will run.
        //
        // Timed and reported: the caller's feedback §13 prints a `device` minus our
        // phases remainder to name the acquire and the present, and says it no longer
        // believes that subtraction is a duration of anything (§11.1's clocks do not
        // mix). Two clock reads a frame is nothing next to being able to name them.
        let acquire_started = Instant::now();
        let bound = self.bind_target(into, viewport)?;
        let acquire = acquire_started.elapsed();

        let query = self.take_pass_query();
        // A replayed frame spent nothing on geometry or staging, and the clock inside a
        // retained encode still holds what the frame that *made* it spent — so the
        // subdivision has to come from the source, not from the clock (ADR 0048).
        let encode_phases = match source {
            EncodeSource::Encoded => encoded.encode_phases.phases(encode_time),
            EncodeSource::Replayed => encoded.encode_phases.replayed(),
        };
        let (execute_wall, mut phases, layer_textures) =
            match self.run_frame(encoded, &bound, upload, query.as_ref(), damage) {
                Ok(ran) => ran,
                Err(error) => return Err(self.abandon_frame(bound, error)),
            };

        // Present before reading instrumentation back: the person sees the frame at
        // the earliest moment, the numbers arrive a map later.
        let mut readback_source: Option<wgpu::Texture> = None;
        let present_started = Instant::now();
        match bound {
            Bound::Acquired(surface_texture) => self.queue.present(surface_texture),
            Bound::Owned(texture) => readback_source = Some(texture),
            Bound::Borrowed(_) => {}
        }
        phases.push(("target acquire", acquire));
        phases.push(("present", present_started.elapsed()));
        phases.extend(encode_phases);
        let (execute, provenance) = timing::read_pass(
            &self.gpu,
            self.timestamps,
            query.as_ref(),
            execute_wall,
            "content pass",
            &mut phases,
        )?;
        // Read, so the buffers are unmapped and the set is the next frame's to use.
        // Reached only on the `?` above succeeding, which is the whole condition: a
        // query whose read failed is dropped here instead, and the frame after it makes
        // a fresh one.
        self.pass_query = query;

        let (payload, readback) = self.resolve_payload(readback_source, viewport)?;

        Ok(Frame {
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
                clip_residue_regions: encoded.clip_residue_regions,
                clip_residue_tiles: encoded.clip_residue_tiles,
                distinct_outlines: encoded.distinct_outlines,
                atlas_entries: u32::try_from(self.atlas.entry_count()).unwrap_or(u32::MAX),
                atlas_distinct_keys: encoded.atlas_distinct_keys,
                atlas_working_set_bytes: encoded.atlas_requested_bytes,
                // Set by the caller of `draw_encoded`, which is where the repack is
                // decided: a frame that has not settled its atlas yet has not repacked
                // it, and a `Frame` may not carry a number that is not true.
                atlas_repacked: false,
                segments: encoded.segments,
                tiles: encoded.tiles,
                commands_culled: encoded.commands_culled,
                layers_culled: encoded.layers_culled,
                bytes_uploaded: upload_bytes,
                layer_textures,
            },
            reports,
            payload,
            encode_source: source,
        })
    }

    /// After a drawn frame that encoded: repack the atlas when this frame's tiles no
    /// longer fit it, and only when repacking can change that (ADR 0024, ADR 0050).
    ///
    /// Answers `true` when the atlas was reset, which is the one event that moves every
    /// tile and so invalidates every [`RetainedScene`] encode keyed on the layout
    /// ([`Counters::atlas_repacked`](crate::frame::Counters::atlas_repacked) reports it).
    ///
    /// Three conditions, and the third is ADR 0050's:
    ///
    /// - **a tile fell through to scratch** — otherwise the atlas is serving the frame;
    /// - **the frame's own working set would fit an empty atlas by bytes**. When the
    ///   distinct keys are simply larger than the cache, resetting throws away the part
    ///   that fits and hits and every frame pays the packing again: measured at 100× on
    ///   the zoom ladder, 6.0 ms of encode against 0.6 ms for keeping what fits;
    /// - **the atlas holds entries this frame did not use**. A repack reclaims exactly
    ///   those, and nothing else: the survivors are re-inserted in the frame's own
    ///   encounter order, which is the order they were inserted in to begin with, so a
    ///   repack of an atlas holding nothing else *provably* reproduces the layout it
    ///   replaced — the same tiles in the same shelves, and the same tile overflowing at
    ///   the end. Taking it anyway was an atlas that reset on every frame of a page it
    ///   could never hold, and a retained encode that never survived one.
    fn settle_atlas(&mut self, encoded: &Encoded) -> bool {
        let (atlas_width, atlas_height) = self.atlas.dimensions();
        let atlas_bytes = u64::from(atlas_width).saturating_mul(u64::from(atlas_height));
        let resident = u32::try_from(self.atlas.entry_count()).unwrap_or(u32::MAX);
        let repack = encoded.atlas_pressure
            && encoded.atlas_requested_bytes <= atlas_bytes
            && resident > encoded.atlas_entries_used;
        if repack {
            self.atlas.reset();
        }
        repack
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
    ) -> Result<(Duration, FramePhases, u32), RenderError> {
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
        let dummy_view = self.ensure_dummy();
        let mask_count = encoded.mask_plans.len();
        // Taken, not borrowed: it belongs to the first frame of its size and to no other.
        let warmed = match self.warmed_layer.take() {
            Some((w, h, pair)) if (w, h) == (width, height) => Some(pair),
            _ => None,
        };
        let mut executor = Executor {
            device: self,
            encoded,
            width,
            height,
            // Empty unless a host called `warm_for` for this size, which puts one pair
            // in it: a plan's pair is otherwise created on its first acquire, so a flat
            // frame creates none at all (ADR 0020).
            pool: LayerPool::warmed(warmed),
            masks: (0..mask_count).map(|_| None).collect(),
            rect_buffer: upload.rect_instances,
            quad_buffer: upload.quad_instances,
            lane_binds: HashMap::new(),
            scratch_view: upload.scratch_view.as_ref().map(|(_, view)| view.clone()),
            dummy_view,
            atlas_view: self.atlas_texture.as_ref().map(|(_, view)| view.clone()),
            first_pass_stamped: false,
            query,
            phases: Vec::new(),
            scissor: patch.map(|(bbox, _)| bbox),
            region: Region::whole(width, height),
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
            let root = executor.render_plan(&mut recorder, 0, None)?;
            executor.patch_to_target(
                &mut recorder,
                &root.view(),
                root.region(),
                &target_view,
                target_format,
                rects,
            )?;
        } else if matches!(damage, DamagePlan::Patch { .. }) {
            // Every damage rect fell outside the target: nothing visible changed,
            // and honouring the list exactly means touching no pixel at all.
        } else if flat {
            // is_flat checked: a flat root holds drawable ops only.
            let root_ops = compose::run_ops(&encoded.root.ops);
            executor.draw_pass(
                &mut recorder,
                &target_view,
                target_format,
                PassLoad::Clear,
                &root_ops,
            )?;
        } else {
            executor.realise_masks(&mut recorder)?;
            let root = executor.render_plan(&mut recorder, 0, None)?;
            executor.blit_to_target(
                &mut recorder,
                &root.view(),
                root.region(),
                &target_view,
                target_format,
            )?;
        }
        executor.end_stamp(&mut recorder, &target_view);
        let layer_textures = u32::try_from(executor.pool.peak()).unwrap_or(u32::MAX);
        let phases = std::mem::take(&mut executor.phases);
        drop(executor);
        if let Some(q) = query {
            recorder.resolve_query_set(&q.set, 0..2, &q.resolve, 0);
            recorder.copy_buffer_to_buffer(&q.resolve, 0, &q.map, 0, 16);
        }
        let execute_wall = compose::submit_and_wait(self, recorder)?;
        Ok((execute_wall, phases, layer_textures))
    }

    /// Phase 4: resolve. Only `Readback` pays anything here (§6.1: this is the cost
    /// that dominated the old backend's offscreen frame, priced separately so §11.1
    /// finally has its answer — and ADR 0022 is what it bought).
    ///
    /// A `Surface` frame has already been presented and a `Texture` frame is where the
    /// caller wanted it, so both return `Payload::None` and a zero: the number is the
    /// truth about what this target cost, not a placeholder.
    fn resolve_payload(
        &self,
        readback_source: Option<wgpu::Texture>,
        viewport: &Viewport<'_>,
    ) -> Result<(Payload, Duration), RenderError> {
        let Some(texture) = readback_source else {
            return Ok((Payload::None, Duration::ZERO));
        };
        let started = Instant::now();
        let raster = readback::read_back(
            &self.gpu,
            &self.queue,
            &texture,
            viewport.width,
            viewport.height,
            self.limits.max_target_size,
        )?;
        Ok((Payload::Raster(raster), started.elapsed()))
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
        source: EncodeSource,
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
                encode_source: source,
            }),
            Target::Surface => Err(RenderError::ZeroSizeTarget { target: "Surface" }),
            Target::Texture(_) => Err(RenderError::ZeroSizeTarget { target: "Texture" }),
        }
    }

    /// Phase 2: create the frame's buffers and textures, sized by phase 1's counts,
    /// and schedule their uploads.
    ///
    /// # Errors
    ///
    /// Whatever the GPU coverage lane refuses: its sheet is a texture like any other
    /// and is bounded by the adapter's dimension.
    fn upload(&mut self, encoded: &Encoded) -> Result<Upload, RenderError> {
        let started = Instant::now();
        // The globals are per plan now (ADR 0036) and made where the pass is recorded,
        // so nothing is charged here for them.
        let mut bytes = 0_u64;
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

        let scratch_view = self.upload_scratch(encoded, &mut bytes)?;
        self.flush_atlas_tiles(&mut bytes);

        Ok(Upload {
            rect_instances,
            quad_instances,
            scratch_view,
            bytes,
            time: started.elapsed(),
        })
    }

    /// The frame's scratch coverage sheet: the CPU lane's bytes uploaded, the GPU
    /// lane's tiles drawn, into one texture.
    ///
    /// One sheet with two producers is the whole of the GPU lane's integration (ADR
    /// 0016): downstream, a coverage quad names a rectangle of *this* texture and has
    /// no idea which lane put the bytes there. The upload goes first because the draw
    /// loads what it finds — a tile the GPU draws covers only its own rectangle, so
    /// the CPU lane's bytes elsewhere on the sheet survive it.
    ///
    /// **Borrowed, not taken.** Both of these used to be moved out of the `Encoded`,
    /// which is what a frame that owns its encode can afford; a retained encode is
    /// replayed by later frames, so the sheet it names has to survive its first upload
    /// (ADR 0048, and ADR 0045's invalidation list named this as the one plumbing
    /// change the design needs).
    fn upload_scratch(
        &mut self,
        encoded: &Encoded,
        bytes: &mut u64,
    ) -> Result<Option<(wgpu::Texture, wgpu::TextureView)>, RenderError> {
        let Some(scratch) = encoded.scratch.as_ref() else {
            return Ok(None);
        };
        let winding = &encoded.winding;
        let mut usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        if !winding.is_empty() {
            // COPY_SRC with it, so a test can read the sheet back and hold the two
            // lanes to a stated difference rather than to a hope.
            usage |= wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        }
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
            usage,
            view_formats: &[],
        });
        if !scratch.data.is_empty() {
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
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        *bytes = bytes.saturating_add(scratch.data.len() as u64);
        if !winding.is_empty() {
            *bytes = bytes.saturating_add(winding.device_bytes());
            crate::winding::render_into(
                &self.gpu,
                &self.queue,
                &self.pipelines,
                &mut self.winding_texture,
                &view,
                winding,
                self.coverage_samples,
                if scratch.data.is_empty() {
                    crate::winding::SheetUse::LaneAlone
                } else {
                    crate::winding::SheetUse::BesideCpuBytes
                },
                self.limits.max_target_size,
            )?;
        }
        Ok(Some((texture, view)))
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

    /// Give up a bound target after a failure, and pass the error through.
    ///
    /// Dropping an acquired-but-unpresented swapchain texture leaves the swapchain
    /// an acquire semaphore no submission will ever wait on, and enough of those
    /// exhaust it — every later acquire times out. Invalidating the surface here
    /// bounds the damage of a post-acquire failure at one lost frame: the next
    /// frame reconfigures, which replaces the swapchain.
    fn abandon_frame(&mut self, bound: Bound<'_>, error: RenderError) -> RenderError {
        if matches!(bound, Bound::Acquired(_)) {
            drop(bound);
            if let Some(surface) = self.surface.as_mut() {
                surface.invalidate();
            }
        }
        error
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

    /// The query set and buffers for one frame's timestamps, when the adapter has
    /// them.
    /// This frame's timestamp query, taken out of the device for the duration.
    ///
    /// **Taken rather than borrowed**, because the frame it belongs to needs `&mut self`
    /// for everything else it does; and taken rather than made, because making one costs
    /// **2.43 ms on a device's first frame** — a `QuerySet` and two sixteen-byte buffers,
    /// which the driver charges for once and then hands back from a pool. That was a
    /// fifth of the eleven milliseconds a first frame pays over its successors
    /// (`QUORRA_FEEDBACK.md` §9), spent on an instrument rather than on the page.
    ///
    /// It goes back at the end of a frame that read it, and does not after one that
    /// could not: a map that failed may leave the buffer mapped, and the next frame's
    /// `map_async` on it would be a validation error rather than a number.
    fn take_pass_query(&mut self) -> Option<PassQuery> {
        self.timestamps?;
        Some(
            self.pass_query
                .take()
                .unwrap_or_else(|| PassQuery::new(&self.gpu)),
        )
    }

    /// The admitted analysis of a resident §7.10.5 program, for the lane that draws it.
    ///
    /// The frame reads it rather than re-deriving it: everything static about a program
    /// was decided once at [`Device::upload_function`], and a second answer computed
    /// per frame would be a second answer.
    pub(crate) fn function_analysis(
        &self,
        program: FunctionId,
    ) -> Option<&crate::function::Analysis> {
        self.resources
            .function(program)
            .map(|stored| &stored.analysis)
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
            let pair = self.rgba_texture("quorra ramp", RAMP_RESOLUTION, 1, &samples);
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
}

/// Dropping a device waits for its warm-up thread, and for nothing else.
///
/// A thread compiling a pipeline is *inside the driver*, and a driver that is torn
/// down — by this device's release, or by `exit()` running the loader's own atexit
/// handlers — while one of its threads is still in there crashes the process. That is
/// not hypothetical: on this machine, thirteen runs in fifteen of a test that built
/// devices and dropped them without rendering died in `quorra-warm-up`, after every
/// test in them had passed (`tests/device_lifecycle.rs`). ADR 0018 has the
/// reproduction and why the wait is bounded by a compile nobody else waits on.
impl Drop for Device {
    fn drop(&mut self) {
        if let Some(handle) = self.warm_up.take() {
            // The result is discarded because a device being dropped has no one left to
            // report to. What the thread ended with is not lost, though: it records that
            // before it leaves, whether it finished, was refused or panicked (ADR 0042),
            // so `warm_up` still answers for as long as the device exists.
            drop(handle.join());
        }
    }
}
