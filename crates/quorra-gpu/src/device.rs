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
//! - `bound` — what a frame draws into, and the contract each of the three targets
//!   must satisfy before it does.
//! - `damage` — ADR 0012's reading of a viewport's damage list, and which target can
//!   honour one.
//! - `rare` — the same for the image and shading quads, which the brief's §0 calls the
//!   rare case.
//! - `record` — phase 3: the route a frame's content takes to the target, recorded
//!   into one submission.
//! - `render` — one frame, from the call to the `Frame`: the phase order, and the
//!   order the refusals are taken in.
//! - `resident` — a resource uploaded once and resident until released, in both the
//!   forms it has.
//! - `staging` — phase 2: the buffers and textures one frame stages before anything is
//!   recorded.
//! - `textures` — the textures a device makes, and the usages each one asks for.

mod binds;
mod bound;
mod damage;
mod ramp;
mod rare;
mod record;
mod render;
mod resident;
mod staging;
mod textures;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::atlas::AtlasStore;
use crate::error::DeviceError;
use crate::layers;
use crate::pipeline::PipelineStore;
use crate::resources::ResourceStore;
use crate::startup::{self, Coverage, Options, PreSteps, StartupTimings, WarmUp};
use crate::surface::SurfaceState;
pub(crate) use crate::timing::PassQuery;
use crate::timing::TimestampSupport;

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

    pub(crate) fn pipelines(&self) -> &PipelineStore {
        &self.pipelines
    }

    pub(crate) fn gpu(&self) -> &wgpu::Device {
        &self.gpu
    }

    pub(crate) fn queue(&self) -> &wgpu::Queue {
        &self.queue
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
