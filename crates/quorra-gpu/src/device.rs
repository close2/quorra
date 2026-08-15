//! The device: adapter, queue, pipelines, and the frames they produce.
//!
//! # The two rules that outrank everything else here
//!
//! **A frame is drawn, or it is refused. There is no third state.** Every allocation
//! in a frame is sized by counting first (`encode.rs`) and checked against a stated
//! budget; every refusal is an [`Err`] naming what was exceeded or unavailable
//! ([`crate::error`]); and a [`Frame`](crate::frame::Frame) is constructed only after
//! every fallible step has succeeded, so a failed frame cannot report itself drawn.
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
mod construct;
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
use std::sync::atomic::AtomicU64;
use std::thread;
use std::time::Duration;

use crate::atlas::AtlasStore;
use crate::pipeline::PipelineStore;
use crate::resources::ResourceStore;
use crate::startup::Coverage;
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
/// [`RetainedScene`](crate::retained::RetainedScene) needs of it: a handle carried to
/// a device other than the one that encoded it must not replay, and two live devices
/// never share a number (ADR 0048).
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
    /// For a tier-3 host ([`Target::Texture`](crate::target::Target::Texture)): the
    /// texture it hands to [`Device::render`] must be created from exactly this device,
    /// and this is where it gets one — along with the queue it will composite with. The
    /// handles are the same reference-counted objects this `Device` renders with, not
    /// copies.
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
