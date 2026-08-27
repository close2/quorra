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
//! # This file, and the eleven modules under it
//!
//! What is left in *this* file is the device itself: the handles it holds for its
//! life, what it will admit ([`Limits`]), and the accessors that hand those out.
//! Everything a device *does* is one of the parts below, listed in the order a device
//! lives rather than in the order they are declared.
//!
//! Each part is private, which is ADR 0051's rule: a caller writes
//! `quorra_gpu::device::Device` and nothing else, so this layout stays ours to change.
//! The names are not links for the same reason — a private module is not in the
//! published documentation, and this list is the only place the structure survives
//! into it.
//!
//! - `construct` — what a device costs to exist and when it is ready: adapter
//!   selection, device creation, the warm-up, and dropping the thread that runs it.
//! - `resident` — a resource uploaded once and resident until released, in both the
//!   forms it has: the validated copy, and the texture a frame draws from.
//! - `ramp` — what one of those forms contains: a colour ramp sampled to texels, which
//!   is arithmetic rather than a device (ADR 0011).
//! - `render` — one frame, from the call to the [`Frame`](crate::frame::Frame): the
//!   phase order, and the order the refusals are taken in.
//! - `damage` — ADR 0012's reading of a viewport's damage list, and which target can
//!   honour one.
//! - `bound` — what a frame draws into, and the contract each of the three targets
//!   must satisfy before it does.
//! - `present` — the surface leaving and coming back, which is the whole of what this
//!   device knows about [`crate::present`] (ADR 0056).
//! - `staging` — phase 2: the buffers and textures one frame stages before anything is
//!   recorded.
//! - `record` — phase 3: the route a frame's content takes to the target, recorded
//!   into one submission.
//! - `binds` — the bind group and the uniform bytes each of the compositor's passes
//!   reads.
//! - `rare` — the same for the image and shading quads, which the brief's §0 calls the
//!   rare case.
//! - `textures` — the textures a device makes, and the usages each one asks for.

mod binds;
mod bound;
mod construct;
mod damage;
mod present;
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
use crate::surface::SurfaceSlot;
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
    /// The glyph atlas this device actually made, in bytes — **not**
    /// [`Options::atlas_budget`], which is a request (ADR 0063).
    ///
    /// The atlas is one R8 texture sized near-square from the budget, with its width
    /// capped at 2048 and both sides clamped to
    /// [`max_target_size`](Limits::max_target_size). So a budget above
    /// `2048 × max_target_size` is granted in part and the rest is silently unavailable —
    /// 32 MiB is the ceiling on an adapter allowing 16 384 texels a side, whatever the
    /// caller asked for. Reported because a caller comparing
    /// [`Counters::atlas_working_set_bytes`] against its own request would be comparing
    /// against the wrong number exactly when the cap bites, and because §5's rule is that
    /// a limit which must exist is discoverable before the frame rather than inferred
    /// from a page that ran slowly.
    ///
    /// [`Options::atlas_budget`]: crate::startup::Options::atlas_budget
    /// [`Counters::atlas_working_set_bytes`]: crate::frame::Counters::atlas_working_set_bytes
    pub atlas_bytes: u64,
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
    /// Area-averaged variants, keyed `(image, x factor, y factor)` (ADR 0089) —
    /// realised once per key for the device's life, exactly as the base textures are.
    reduced_textures: HashMap<(u32, u32, u32), (wgpu::Texture, wgpu::TextureView)>,
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
    /// How many threads a frame's geometry may use, as the host stated it
    /// ([`Options::encode_threads`]). Clamped where the device is built, so the encoder
    /// reads a number rather than a request.
    encode_threads: usize,
    coverage_samples: u32,
    /// The GPU lane's winding target, kept across frames (ADR 0016's measurement).
    winding_texture: crate::winding::WindingTexture,
    /// The compute coverage lane's pipelines, compiled on the first frame that takes
    /// the lane and kept — never on the startup path (ADR 0080).
    compute_pipelines: Option<crate::compute::Pipelines>,
    /// The compute lane's resident segments: every outline it has drawn, uploaded
    /// once (ADR 0081).
    segment_arena: crate::compute::SegmentArena,
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
    /// The compute lane's two pass queries — count, and emit+deposit — kept for the
    /// device's life exactly as [`Self::pass_query`] is, and absent on the same
    /// conditions. They exist because the lane's dispatches run in submissions of
    /// their own *before* the content pass, so its device time was invisible to the
    /// one query the frame had: the caller's ADR 0084 carried ~25 ms of "unattributed"
    /// per worst-page frame, and most of it was this lane's.
    compute_queries: Option<crate::compute::ComputeQueries>,
    /// The surface, and who has it: this device, a [`Presenter`](crate::present::Presenter)
    /// it handed out, or nobody because there never was one (ADR 0056).
    surface: SurfaceSlot,
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

    /// The pipeline cache: the passes in modules of their own, and the tests that
    /// drive one of those passes without a frame around it, both reach it here.
    pub(crate) fn pipelines(&self) -> &PipelineStore {
        &self.pipelines
    }
}
