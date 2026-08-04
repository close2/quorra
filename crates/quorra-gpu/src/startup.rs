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
//! What is *not* here, deliberately: a backend-set knob. Restricting the instance to
//! Vulkan halves `Instance::new` and gives every millisecond back in
//! `request_adapter` — the caller measured both halves on this machine's three
//! adapters, and the total is the invariant (their feedback §8.3). A knob whose only
//! evidence is a plausible argument is not a knob we add.

use std::time::Duration;

use crate::error::DeviceError;

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
    /// Compiling the warm pipeline set, on the background thread. `None` while that
    /// is still in flight — poll [`Device::is_warm`], or read this again later. Not
    /// part of [`StartupTimings::blocking_total`]: nothing blocks on it.
    ///
    /// [`Device::is_warm`]: crate::device::Device::is_warm
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
#[must_use]
pub fn create_instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle())
}

/// Choose the adapter: the [`Options::adapter`] filter when there is one, wgpu's own
/// preference when there is not.
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
