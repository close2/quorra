//! The surface, off the device: presenting a finished raster from a thread that is not
//! the one rendering (ADR 0056).
//!
//! # The number this exists for
//!
//! The caller measured a frame of a 58 009-command page at 4 454.9 ms over 24 frames,
//! of which **`execute` — the device's own timestamps — was 6.7 ms.** The graphics
//! device is idle for 99.85 % of such a frame; what makes the frame long is a processor
//! running on the calling thread. So the seam that buys anything is not "submit and
//! poll": it is that the surface stops being the same object's business as the caches,
//! so that a window can be re-drawn from the last finished raster **while** the next
//! one is being encoded, on another thread, holding the `&mut Device` that a present
//! would otherwise have to wait for.
//!
//! # What a presenter is, and what it is not
//!
//! It is the surface, its swapchain, and the one pipeline that puts a texture on it. It
//! is **not** a renderer: nothing here encodes a scene, allocates from scene-derived
//! arithmetic, or touches the glyph atlas — [`Presenter::present`] samples rasters some
//! earlier frame already finished. That is why nothing here can move a pixel of a page,
//! and why the corpus gate and the oracle — both `Target::Readback`, neither with a
//! surface — cannot reach any of it.
//!
//! # Threading, and why it is sound
//!
//! [`Presenter`] is `Send`, and this is a claim about `wgpu` 30 rather than about our
//! own code. Everything a presenter holds beside its own numbers is a `wgpu` handle —
//! `Surface<'static>`, `Device`, `Queue`, `Sampler` — and an `Arc<PipelineStore>` whose
//! contents are `ShaderModule`s and `RenderPipeline`s behind a `Mutex`. `wgpu` asserts
//! `Send + Sync` for every one of those types on native targets, in its own source
//! (`static_assertions::assert_impl_all!` under its `send_sync` cfg, which is `not(wasm32)`),
//! and `Sync` in safe Rust *is* the statement that concurrent `&self` use is sound.
//! This crate is `#![forbid(unsafe_code)]`, so nothing here can weaken that.
//!
//! Two caveats stated rather than discovered, both `wgpu`'s and neither ours to fix:
//! **surface creation** panics off the main thread on macOS/Metal (`wgpu`'s own
//! `SurfaceTarget` documentation), which is why a presenter is *detached* from a device
//! the host built where its window lives rather than constructed on the thread that
//! will use it; and a host that presents from two threads at once is asking two
//! swapchains' worth of questions of one swapchain, which is why [`Presenter::present`]
//! takes `&mut self` and there is exactly one presenter per device.
//!
//! # This module's parts
//!
//! Private, re-exported here, which is ADR 0051's rule: `quorra_gpu::present::Layer` is
//! a path a caller may write and `quorra_gpu::present::layer::Layer` is not.
//!
//! - `layer` — what a host hands over, and the contract it is held to.
//! - `pass` — what one present records, and the uniform `present.wgsl` reads.

mod layer;
mod pass;

use std::sync::Arc;
use std::time::{Duration, Instant};

pub use layer::Layer;

use crate::error::RenderError;
use crate::pipeline::{Kind, PipelineStore};
use crate::surface::SurfaceState;

/// What the last [`Presenter::present`] cost, and how each number was obtained.
///
/// **Every duration here is a host wall clock**, and each says so in its name. There is
/// no timestamp-query number in this type on purpose: a query has to be resolved and
/// mapped to be read, which is a stall on the thread whose freedom from stalls is the
/// whole point of the split, and a duration reported as if it were a device measurement
/// is the one thing §8 of the brief forbids outright. What is exact here are the two
/// counts — `layers` and `reconfigured` — and they are the numbers to gate on. See
/// [`TimingProvenance`](crate::frame::TimingProvenance) for the same rule stated on a
/// frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentCost {
    /// How many layers were drawn. Exact.
    pub layers: usize,
    /// Whether the swapchain was configured — replaced — before this acquire. Exact,
    /// and the one fact that makes a present cost unlike its neighbours: a resize, a
    /// surface that reported itself suboptimal or outdated, or a recovery from a
    /// timeout all land here.
    pub reconfigured: bool,
    /// What compiling the present pipeline cost, when this present was the one that
    /// compiled it. `None` — the ordinary case — means it was already built, which for
    /// a device constructed with a surface is what the warm-up thread did (ADR 0043,
    /// ADR 0056). A wall clock.
    pub compiled: Option<Duration>,
    /// Acquiring the swapchain texture, including any configure. A wall clock, and the
    /// number that carries the wait for a refresh under `Fifo`.
    pub acquire_wall: Duration,
    /// Recording the pass and submitting it — not executing it, which happens on the
    /// device afterwards. A wall clock.
    pub record_wall: Duration,
    /// Handing the texture to the presentation engine. A wall clock.
    pub present_wall: Duration,
}

/// The surface, its swapchain, and the one pipeline that puts a texture on it — held
/// apart from the [`Device`](crate::device::Device) that made it, so that presenting
/// and rendering can happen at the same time on different threads.
///
/// Obtained from [`Device::detach_presenter`](crate::device::Device::detach_presenter)
/// and returned by [`Device::attach_presenter`](crate::device::Device::attach_presenter);
/// there is never more than one for a device, and while it is out that device refuses
/// [`Target::Surface`](crate::target::Target::Surface) by name
/// ([`RenderError::PresenterDetached`]).
///
/// **Dropping one loses the window's surface**, and no error path in this crate does
/// that to a host: attaching to the wrong device hands the presenter back inside the
/// refusal ([`ForeignPresenter`]) rather than consuming it.
pub struct Presenter {
    /// Which device handed this out. A retained encode may not be replayed through
    /// another device (ADR 0048) and a surface may not be attached to one either; the
    /// number is the same monotonic device id, for the same reason — two live devices
    /// never share one.
    device_id: u64,
    gpu: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: Arc<PipelineStore>,
    /// The device's own filtering sampler, shared rather than made again: it is a
    /// descriptor with no state, and one of them is what `ImageFilter::Linear` means
    /// everywhere in this crate.
    sampler: wgpu::Sampler,
    surface: SurfaceState,
    /// The size the next present configures and acquires at. `None` until the host says
    /// — see [`RenderError::PresenterUnsized`].
    size: Option<(u32, u32)>,
    /// What the last *successful* present cost. A refused present leaves it alone,
    /// because it is a statement about a present that happened.
    last: Option<PresentCost>,
}

impl std::fmt::Debug for Presenter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Presenter")
            .field("device_id", &self.device_id)
            .field("surface", &self.surface)
            .field("size", &self.size)
            .field("last", &self.last)
            .finish_non_exhaustive()
    }
}

impl Presenter {
    /// The pieces a device hands over. Not public: a presenter comes from a device or
    /// it comes from nowhere, because the surface's format was negotiated against the
    /// adapter that device chose and nothing outside can learn it (`wgpu::Device`
    /// offers `adapter_info()` and no adapter).
    pub(crate) fn new(
        device_id: u64,
        gpu: wgpu::Device,
        queue: wgpu::Queue,
        pipelines: Arc<PipelineStore>,
        sampler: wgpu::Sampler,
        surface: SurfaceState,
    ) -> Self {
        // The size a device already configured for, when it presented a frame before
        // handing the surface over. `None` otherwise, and then the host says.
        let size = surface.configured_size();
        Self {
            device_id,
            gpu,
            queue,
            pipelines,
            sampler,
            surface,
            size,
            last: None,
        }
    }

    /// Which device this presenter belongs to.
    pub(crate) fn device_id(&self) -> u64 {
        self.device_id
    }

    /// Give the surface back to the device that owns it.
    pub(crate) fn into_surface(self) -> SurfaceState {
        self.surface
    }

    /// Put `layers` on the window, in order, each under its own placement and filter,
    /// over a window cleared to transparency.
    ///
    /// Every layer must be a texture **this presenter's device** rendered into through
    /// [`Target::Texture`](crate::target::Target::Texture), and must additionally carry
    /// `TEXTURE_BINDING` usage so that it can be sampled — [`Layer`] states the whole
    /// contract and [`LayerProblem`](crate::error::LayerProblem) is how each part of it
    /// is refused.
    ///
    /// An empty slice is a legitimate present: it clears the window, which is what a
    /// host with nothing to show asked for.
    ///
    /// # Errors
    ///
    /// [`RenderError::PresenterUnsized`] before [`Presenter::resize`] has ever been
    /// called on a surface no frame had configured; [`RenderError::ZeroSizeTarget`] for
    /// a window with no pixels (a minimised one, typically);
    /// [`RenderError::LayerRefused`] naming the layer and what did not hold;
    /// [`RenderError::PipelineUnavailable`] when this adapter refuses the present
    /// pipeline; and [`RenderError::SurfaceUnavailable`] when the swapchain cannot
    /// provide a texture right now — which is a retry, and says which kind.
    ///
    /// **On every one of them nothing was presented**, and [`Presenter::last`] still
    /// describes the last present that happened.
    pub fn present(&mut self, layers: &[Layer<'_>]) -> Result<(), RenderError> {
        let (width, height) = self.size.ok_or(RenderError::PresenterUnsized)?;
        if width == 0 || height == 0 {
            return Err(RenderError::ZeroSizeTarget { target: "Surface" });
        }

        // Everything that can refuse, before the acquire. A swapchain texture acquired
        // and then dropped unpresented leaves a semaphore no submission will wait on,
        // and enough of those wedge the surface permanently — the caller measured that.
        let (pipeline, compiled) = self.pipelines.get(Kind::Present, self.surface.format())?;
        let binds = pass::bind_layers(
            &self.gpu,
            &self.queue,
            &self.sampler,
            &self.pipelines.present_layout(),
            layers,
            (width, height),
        )?;

        let acquire_started = Instant::now();
        let target = self.surface.acquire(&self.gpu, width, height)?;
        let acquire_wall = acquire_started.elapsed();

        let record_started = Instant::now();
        let view = target
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        pass::record(&self.gpu, &self.queue, &pipeline, &view, &binds);
        let record_wall = record_started.elapsed();

        let present_started = Instant::now();
        self.queue.present(target);
        self.last = Some(PresentCost {
            layers: layers.len(),
            reconfigured: self.surface.reconfigured(),
            compiled,
            acquire_wall,
            record_wall,
            present_wall: present_started.elapsed(),
        });
        Ok(())
    }

    /// Tell the presenter how big the window is now.
    ///
    /// **A resize is not a frame**: nothing is configured, acquired or drawn here, and
    /// the swapchain follows the window at the next [`Presenter::present`] — which is
    /// also the call that can refuse, so a resize has nothing to return. A host may call
    /// this as often as its window system tells it something, including with the size it
    /// already had.
    ///
    /// A zero-size window is a state, not an error (a minimised one is exactly this);
    /// presenting at that size is what refuses, by name.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = Some((width, height));
    }

    /// The size the next present will acquire at, once one is known.
    #[must_use]
    pub fn size(&self) -> Option<(u32, u32)> {
        self.size
    }

    /// What the last successful present cost, or `None` before there has been one.
    ///
    /// `Option` rather than a zeroed [`PresentCost`] deliberately: a cost of nothing and
    /// a present that never happened are different things, and a type that cannot tell
    /// them apart is a type that says something untrue about itself.
    #[must_use]
    pub fn last(&self) -> Option<PresentCost> {
        self.last
    }
}

/// A [`Presenter`] offered to a device that is not the one it came from — and the
/// presenter itself, handed back.
///
/// Handed back because dropping it would destroy the window's surface, and a host that
/// mixed up two devices has a bug to fix rather than a window to lose. Recovering it is
/// [`ForeignPresenter::into_presenter`].
#[derive(Debug, thiserror::Error)]
#[error(
    "this presenter was detached from device {presenter_device} and cannot be attached to \
     device {device}; a surface belongs to the device whose adapter negotiated its format"
)]
pub struct ForeignPresenter {
    /// Boxed because every `Ok` of [`Device::attach_presenter`] would otherwise be as
    /// large as a whole presenter: a `Result` is as big as its widest arm, and the arm
    /// that never happens should not be what sizes the one that does.
    ///
    /// [`Device::attach_presenter`]: crate::device::Device::attach_presenter
    presenter: Box<Presenter>,
    /// The device the presenter came from.
    presenter_device: u64,
    /// The device it was offered to.
    device: u64,
}

impl ForeignPresenter {
    pub(crate) fn new(presenter: Presenter, device: u64) -> Self {
        Self {
            presenter_device: presenter.device_id(),
            presenter: Box::new(presenter),
            device,
        }
    }

    /// Take the presenter back out, to attach it to the device it does belong to.
    #[must_use]
    pub fn into_presenter(self) -> Presenter {
        *self.presenter
    }
}
