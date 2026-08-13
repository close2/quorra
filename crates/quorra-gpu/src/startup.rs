//! How a device comes to exist: what configures construction, how the adapter is
//! chosen, and what each step of it cost.
//!
//! §7 of the brief and CLAUDE.md's "startup is a first-class requirement" are this
//! module's whole subject. The caller's decision that page one goes to the graphics
//! device (their ADR 0179) put bring-up on their time-to-first-page, so the two
//! obligations here are sharper than they were at M1:
//!
//! - **Every reported number names exactly one step.** [`StartupTimings`] has a field
//!   per thing that can regress independently — the driver loader, surface creation,
//!   physical-device enumeration, `request_device` — because a host watching one
//!   number that measures three cannot say which moved. That is not hypothetical: the
//!   single `adapter_enumeration` this replaces measured all three of the first ones.
//! - **What needs no window may be started before there is one.** [`create_instance`]
//!   is the instance quorra would have made for itself, exposed so a host can make it
//!   on a thread at `main`'s first line and hand it to
//!   [`Device::for_surface_with_instance`](crate::device::Device::for_surface_with_instance).
//!   `wgpu::Instance` is `Send + Sync`; the caller measured ~20 ms of a 145 ms launch
//!   in the overlap.
//!
//! - **Which driver stack talks to the hardware is the host's to say.**
//!   [`create_instance_with`] takes the backend set; [`create_instance`] is it with
//!   `Backends::all()` and is unchanged. This is an escape hatch from a driver, not a
//!   speed knob — restricting the instance to Vulkan halves `Instance::new` and gives
//!   every millisecond back in `request_adapter`, which the caller measured (their
//!   feedback §8.3), so the total is the invariant. What it *is* for is the machine in
//!   their §12, where wgpu reached an Intel Vulkan driver that crashed and nothing
//!   could ask for the DX12 one. ADR 0017 records the shape and the two silences that
//!   go with it: no backend field in [`Options`], and no `WGPU_BACKEND`.

use std::time::Duration;

use crate::error::{DeviceError, PipelineProblem};

/// Where the background pipeline warm-up has got to.
///
/// One running state and three ways of being finished, for the reason §5 gives about
/// frames: a caller that waits for the warm set — by polling [`Device::warm_up`] or by
/// blocking in [`Device::wait_until_warm`] — must be able to learn that it is never
/// coming, and only one of the three finished states means it arrived.
/// [`Device::is_warm`] is this question narrowed to that one.
///
/// [`Device::warm_up`]: crate::device::Device::warm_up
/// [`Device::wait_until_warm`]: crate::device::Device::wait_until_warm
/// [`Device::is_warm`]: crate::device::Device::is_warm
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarmUp {
    /// Still compiling. The device renders correctly meanwhile, compiling what a frame
    /// needs on the spot.
    Running,
    /// The warm set exists; the duration is what compiling it cost, and it is what
    /// [`StartupTimings::pipeline_compilation`] reports.
    Warm(Duration),
    /// This adapter refused a shader or a pipeline of the warm set, by name. Nothing
    /// retries it, and a frame needing that pipeline is refused with the same reason.
    Refused(PipelineProblem),
    /// The warm-up thread ended without an answer, because it panicked.
    ///
    /// Reachable, and recorded rather than left as a wait nobody will end: this crate
    /// pops a `wgpu` error scope for validation, but an out-of-memory or an internal
    /// error is neither, and those still reach `wgpu`'s uncaptured-error handler —
    /// which panics. A device in this state has no warm set and no reason to give for
    /// it; the frame that next needs one of those pipelines compiles it itself and
    /// reports whatever happens then.
    Abandoned,
}

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

/// Which producer makes coverage bytes for fills and strokes.
///
/// The two lanes hand the same artefact — an R8 tile in the frame's scratch sheet — to
/// the same quad lane, so nothing downstream of coverage can tell them apart. What
/// differs is where the cost falls, and the curves are opposite (ADR 0016).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Coverage {
    /// The CPU scanline rasteriser of ADR 0008, with the glyph atlas in front of it.
    ///
    /// Coverage is the **exact** area of the pixel a shape covers, 256 levels, and a
    /// page of text at reading size costs almost nothing because the atlas answers
    /// most of it. Its weakness is magnification: past `MAX_GLYPH_DIM` a glyph never
    /// enters the atlas and is rasterised again on every frame.
    #[default]
    Cpu,
    /// Outline triangles rasterised by the device, winding accumulated in a float
    /// target (ADR 0016 — Wallace's method).
    ///
    /// **Nothing in it depends on the magnification**: cubics become quadratics once,
    /// at upload, so a frame at 100× re-uses what a frame at 1× built, and a zoom
    /// gesture — where every cached tile is cold on every frame — costs the same as
    /// standing still. Its weakness is the other end: coverage is sampled rather than
    /// exact ([`Options::coverage_samples`] levels, not 256), and there is no atlas in
    /// front of it, so a dense page of small text pays per glyph per frame.
    ///
    /// Commands under a non-rectangular clip still take the CPU lane, because the
    /// residue multiply is CPU-side; both kinds of tile share one sheet.
    Gpu,
}

/// The GPU lane's default sample count: sixteen, on a 4×4 grid.
///
/// Seventeen coverage levels. Wallace's article reached eight samples by packing them
/// into the bits of one byte; a float target has no such ceiling, and sixteen is where
/// a half-covered pixel lands on exactly 128 while the pass still runs four times.
pub const DEFAULT_COVERAGE_SAMPLES: u32 = 16;

/// Construction options.
///
/// A plain value: budgets, an adapter filter, the glyph quantum. No `wgpu` handle
/// lives here on purpose — a hoisted [`wgpu::Instance`] is an argument to the
/// constructor that uses it ([`Device::headless_with_instance`] and
/// [`Device::for_surface_with_instance`]), so that a host cloning an `Options`
/// around is never accidentally sharing a device-adjacent resource.
///
/// [`Device::headless_with_instance`]: crate::device::Device::headless_with_instance
/// [`Device::for_surface_with_instance`]: crate::device::Device::for_surface_with_instance
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
    /// allocated — including, for a [`Target::Surface`] frame, before the swapchain
    /// texture is acquired, so a refusal costs the surface nothing.
    ///
    /// [`RenderError::FrameBudgetExceeded`]: crate::error::RenderError::FrameBudgetExceeded
    /// [`Target::Surface`]: crate::target::Target::Surface
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
    /// Which lane produces coverage ([`Coverage`]); [`Coverage::Cpu`] by default,
    /// which is the lane whose bytes are exact and whose output the caller's CPU
    /// oracle agrees with.
    pub coverage: Coverage,
    /// Subdivide [`Timings::encode`] into geometry, staging and recording, reported
    /// through [`Timings::phases`] (the caller's feedback §13; ADR 0023).
    ///
    /// **Off by default, because the measurement is not free.** Encode's parts
    /// interleave per command, so the subdivision reads the clock at each seam: about
    /// 0.2 ms over a page of 5 933 commands, which is three times the whole encode of a
    /// page of rectangles and about 1% of a page of paths. A host that traces frames
    /// turns it on for the trace; a host that does not pays an `Option` check.
    ///
    /// [`Timings::encode`]: crate::frame::Timings::encode
    /// [`Timings::phases`]: crate::frame::Timings::phases
    pub instrument_encode: bool,
    /// How many samples the GPU lane takes per pixel, rounded down to a square and
    /// clamped to 4..=64 ([`DEFAULT_COVERAGE_SAMPLES`]).
    ///
    /// Coverage has `samples + 1` levels rather than 256, so this is the quality knob
    /// and it costs **time, not memory**: four samples fit one `rgba16float` texel, and
    /// a frame runs the pass once per group of four. Ignored by [`Coverage::Cpu`],
    /// whose coverage is analytic.
    pub coverage_samples: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            adapter: None,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            max_resource_bytes: DEFAULT_MAX_RESOURCE_BYTES,
            atlas_budget: DEFAULT_ATLAS_BUDGET,
            glyph_quantum: Some(DEFAULT_GLYPH_QUANTUM),
            coverage: Coverage::Cpu,
            coverage_samples: DEFAULT_COVERAGE_SAMPLES,
            instrument_encode: false,
        }
    }
}

/// What startup cost, one field per step that can regress on its own (§7), so a
/// regression can be attributed rather than argued about. Gated in CI from the commit
/// that first produced it.
///
/// The four blocking steps happen in field order and are summed by
/// [`StartupTimings::blocking_total`]; `pipeline_compilation` is not among them
/// because no constructor waits for it.
#[derive(Debug, Clone, Copy)]
pub struct StartupTimings {
    /// `wgpu::Instance::new`: the driver loader, and on this machine the larger half
    /// of what a single "adapter enumeration" number used to hide.
    ///
    /// `None` when the caller supplied the instance
    /// ([`Device::headless_with_instance`], [`Device::for_surface_with_instance`]) —
    /// the step happened, but not here, and reporting zero for work someone else
    /// timed would be a number that lies about what it measured.
    ///
    /// [`Device::headless_with_instance`]: crate::device::Device::headless_with_instance
    /// [`Device::for_surface_with_instance`]: crate::device::Device::for_surface_with_instance
    pub instance_creation: Option<Duration>,
    /// Turning the window handle into a `wgpu::Surface`. Genuinely zero for a
    /// headless device, where the step does not happen at all — which is why this is
    /// a `Duration` and `instance_creation` is an `Option`.
    pub surface_creation: Duration,
    /// Physical-device enumeration and the choice among the results: `request_adapter`,
    /// or `enumerate_adapters` plus the [`Options::adapter`] filter. A different cause
    /// from instance creation — the loader versus the devices — and it moves for
    /// different reasons.
    pub adapter_selection: Duration,
    /// `request_device`: getting a queue on the chosen adapter.
    pub device_creation: Duration,
    /// Compiling the warm pipeline set, on the background thread. `None` while that is
    /// still in flight — poll [`Device::warm_up`], or read this again later — and
    /// `None` for good if that warm-up was refused, which is why the state to poll is
    /// [`WarmUp`] rather than a boolean. Not part of
    /// [`StartupTimings::blocking_total`]: nothing blocks on it.
    ///
    /// [`Device::warm_up`]: crate::device::Device::warm_up
    pub pipeline_compilation: Option<Duration>,
}

impl StartupTimings {
    /// What the constructor actually blocked for: instance creation when it was ours,
    /// plus surface creation, adapter selection and device creation.
    ///
    /// Pipeline compilation is excluded because no constructor waits for it — a
    /// caller that chooses to wait ([`Device::wait_until_warm`]) is adding a cost, not
    /// discovering one.
    ///
    /// [`Device::wait_until_warm`]: crate::device::Device::wait_until_warm
    #[must_use]
    pub fn blocking_total(&self) -> Duration {
        self.instance_creation
            .unwrap_or(Duration::ZERO)
            .saturating_add(self.surface_creation)
            .saturating_add(self.adapter_selection)
            .saturating_add(self.device_creation)
    }
}

/// The two blocking steps a constructor performs before adapter selection, carried
/// into the shared build path so each one keeps its own number.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PreSteps {
    /// `None` when the instance was the caller's.
    pub instance_creation: Option<Duration>,
    pub surface_creation: Duration,
}

impl PreSteps {
    /// A device built on an instance the caller made, with no surface yet.
    pub(crate) const BORROWED_INSTANCE: Self = Self {
        instance_creation: None,
        surface_creation: Duration::ZERO,
    };
}

/// The `wgpu::Instance` quorra creates for itself, exposed so a host can create it
/// early.
///
/// Instance creation needs no window, no surface and no event loop, so it can run on
/// a thread started at `main`'s first line while the document is read and the window
/// opened; `wgpu::Instance` is `Send + Sync`, so that thread can hand it back. The
/// caller measured the overlap at roughly 20 ms of a 145 ms launch (their feedback
/// §8.2). Hand the result to
/// [`Device::for_surface_with_instance`](crate::device::Device::for_surface_with_instance).
///
/// Use this rather than constructing a `wgpu::Instance` by hand: the descriptor is
/// the one quorra's own constructors use, and surface creation is only guaranteed
/// against an instance made the same way.
///
/// **One instance per process when measuring.** A second instance in the same process
/// finds the driver loader warm and reports a fraction of the true cost — the caller
/// spent a first version of its bring-up harness on exactly that mistake.
///
/// Every backend `wgpu` was built with is loaded. To name a subset — because a machine
/// has a driver that must be avoided — use [`create_instance_with`].
#[must_use]
pub fn create_instance() -> wgpu::Instance {
    create_instance_with(wgpu::Backends::all())
}

/// [`create_instance`], restricted to the backends the host names: `Backends::DX12` on
/// a Windows machine whose Vulkan driver crashes, `Backends::VULKAN` on a machine
/// whose GL driver does.
///
/// **Why this is a parameter rather than a preference.** One GPU is enumerated once per
/// backend that can drive it, under the *device's* name both times, so
/// [`Options::adapter`] cannot express "this GPU, through DX12" — it selects hardware,
/// and the question here is which driver stack talks to it. With no restriction the
/// choice falls to wgpu's hub order, where Vulkan precedes DX12; that is how the
/// caller's project owner reached a crashing Intel Vulkan driver on Windows with no way
/// to ask for the other one (their feedback §12). The backend set belongs to the
/// instance, which is made before an [`Options`] exists, so it is an argument here and
/// nowhere else (ADR 0017).
///
/// **Not a speed knob**, and the measurement is in ADR 0014 §3: restricting to Vulkan
/// halves `Instance::new` and gives every millisecond of it back in `request_adapter`.
/// A host with no driver to avoid should call [`create_instance`].
///
/// **The environment is not consulted**, here or in [`create_instance`] — this argument
/// is the only route, deliberately (ADR 0017). A host that wants `WGPU_BACKEND` honoured
/// can say so in one line, and keep its own command line above it:
///
/// ```no_run
/// # use quorra_gpu::{create_instance, create_instance_with, wgpu};
/// let from_flag: Option<wgpu::Backends> = None; // whatever `--backend` parsed to
/// let instance = match from_flag.or_else(wgpu::Backends::from_env) {
///     Some(backends) => create_instance_with(backends),
///     None => create_instance(),
/// };
/// ```
///
/// # Naming a set this machine cannot supply
///
/// Is not an error here — an instance with no usable backend is constructible, and
/// nothing has been asked of it yet. It becomes [`DeviceError::NoAdapter`] at the
/// constructor that uses it ([`Device::headless_with_instance`],
/// [`Device::for_surface_with_instance`]), with an **empty** `available` list; on a
/// machine that visibly has a GPU, that emptiness is the signature of this mistake
/// rather than of a broken driver.
///
/// [`Device::headless_with_instance`]: crate::device::Device::headless_with_instance
/// [`Device::for_surface_with_instance`]: crate::device::Device::for_surface_with_instance
#[must_use]
pub fn create_instance_with(backends: wgpu::Backends) -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

/// Choose the adapter: the [`Options::adapter`] filter when there is one, wgpu's own
/// preference when there is not.
///
/// The `Backends::all()` passed to `enumerate_adapters` is not a second backend
/// decision: `wgpu::Instance` only holds the backends it was built with, so the mask
/// means "everything this instance has" and [`create_instance_with`]'s restriction is
/// already inside it.
pub(crate) fn select_adapter(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'static>>,
    options: &Options,
) -> Result<wgpu::Adapter, DeviceError> {
    match &options.adapter {
        Some(pattern) => {
            let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
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
            let available = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
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
