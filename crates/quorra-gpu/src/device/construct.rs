//! What a device costs to exist and when it is ready: adapter selection, device
//! creation, the background warm-up, and the numbers §7 asks all of them be reported
//! in.
//!
//! **Construction blocks on the two things a device *is*** — an adapter and a device —
//! and on nothing else. The pipelines compile on a thread nobody waits for, which is
//! why [`Device::headless`] returns before the warm set exists and [`Device::is_warm`]
//! and [`Device::warm_up`] answer for it afterwards. The second of those exists
//! because §5's rule reaches startup as well: a caller waiting for a warm set has to
//! be able to learn that it is never coming, and a boolean cannot say that.
//!
//! Four entry points and one body. Two are headless and two present to a window; the
//! difference within each pair is only who created the [`wgpu::Instance`], which is the
//! startup lever the caller measured 20 ms of a 145 ms launch inside (their feedback
//! §8.2). What cannot be hoisted is stated where it is claimed rather than left to be
//! discovered: `request_adapter` takes the surface, so adapter selection is genuinely
//! downstream of the window.
//!
//! Dropping a device is here too, because it is the far end of the same thread: one
//! compiling inside the driver may not outlive the device it compiles for (ADR 0018).

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::{Device, Limits, NEXT_DEVICE_ID, StartupSteps};
use crate::atlas::AtlasStore;
use crate::error::DeviceError;
use crate::layers;
use crate::pipeline::PipelineStore;
use crate::resources::ResourceStore;
use crate::startup::{self, Options, PreSteps, StartupTimings, WarmUp};
use crate::surface::{SurfaceSlot, SurfaceState};
use crate::timing::{PassQuery, TimestampSupport};

/// What asking an adapter for a device produced, and what the asking cost.
struct Requested {
    gpu: wgpu::Device,
    queue: wgpu::Queue,
    timestamps: Option<TimestampSupport>,
    /// `request_device` alone, which §7 wants reported apart from adapter selection.
    creation: Duration,
}

/// Ask the adapter for a device: which features are wanted, which limits are asked
/// for, and what the call cost.
///
/// `adapter` is named rather than borrowed from the caller's `info` because the error
/// has to say which adapter refused.
fn request_device(adapter: &wgpu::Adapter, adapter_name: &str) -> Result<Requested, DeviceError> {
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
        adapter: adapter_name.to_owned(),
        source,
    })?;
    let creation = device_started.elapsed();

    let timestamps = required_features
        .contains(wgpu::Features::TIMESTAMP_QUERY)
        .then(|| TimestampSupport {
            period: queue.get_timestamp_period(),
        });

    Ok(Requested {
        gpu,
        queue,
        timestamps,
        creation,
    })
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
            .transpose()?
            .map_or(SurfaceSlot::Headless, SurfaceSlot::Held);

        let Requested {
            gpu,
            queue,
            timestamps,
            creation: device_creation,
        } = request_device(&adapter, &info.name)?;

        let pipelines = PipelineStore::new(gpu.clone());
        // A surface device warms the presenting lanes in the surface's own format too,
        // so its first frame does not compile them inside itself (ADR 0043).
        let warm_up = pipelines.spawn_warm_up(surface_state.format());

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
            // A request becomes a number here: zero means "one", and nothing above the
            // machine's own parallelism can be honoured by the machine.
            encode_threads: options
                .encode_threads
                .max(1)
                .min(std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)),
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
