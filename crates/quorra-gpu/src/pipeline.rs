//! Pipelines and shaders: how few, how late, and what happens when one is refused.
//!
//! §7 of the brief makes this module a startup-latency problem before it is a
//! rendering problem. The caller renders page one on its CPU backend *while we
//! initialise*, so what we cost before the first frame is what decides whether the
//! handover is invisible. The rules, from the brief and `doc/PLAN.md` §1.8:
//!
//! - **No pipeline compilation on the critical path of device construction.**
//!   `PipelineStore::new` creates nothing on the GPU; the warm set compiles on a
//!   background thread (`PipelineStore::spawn_warm_up`), and its duration lands in
//!   `StartupTimings::pipeline_compilation`. Nobody waits on that thread — but the
//!   device *owns* it and joins it when dropped, because a thread inside the driver
//!   may not outlive the device it compiles for (ADR 0018).
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
//! # A pipeline that cannot be built is refused, not survived
//!
//! `wgpu` reports a shader or pipeline failure *out of band*: the constructor hands
//! back a handle either way and the error goes to the device's uncaptured-error
//! handler, whose default is to panic — on whatever thread was compiling, which for
//! the warm set is a thread nobody is listening to. Every compile here therefore runs
//! inside a validation error scope ([`captured`]), a captured failure becomes a
//! [`PipelineProblem`], and [`PipelineStore::get`] is fallible so the frame that needed
//! the pipeline is refused by name (ADR 0042). What the warm-up thread ends with is
//! recorded in [`WarmUp`] on **every** exit path, so [`PipelineStore::wait_until_warm`]
//! always returns.
//!
//! Fallibility is what the module's three files are split along: `layouts.rs` is the
//! half of a pipeline no adapter can refuse, `spec.rs` is what each [`Kind`] *is*, and
//! this file is the store — its one lock, its laziness and its warm-up.
//!
//! # Shaders are code, and this project's rules apply to them
//!
//! WGSL lives in `src/shaders/`, and every function implementing a normative
//! requirement carries its clause number in a comment — a WGSL blend function is not
//! exempt from CLAUDE.md principle 5 just because it is not Rust. A shader whose
//! invariants are not stated beside it is write-only code (principle 4); each shader
//! states its coverage definition and its determinism argument inline.

mod layouts;
mod spec;

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::PipelineProblem;
use crate::startup::WarmUp;
use layouts::Layouts;
pub(crate) use spec::Kind;
use spec::Spec;

/// The format the warm-up thread compiles for: the readback and host-texture format,
/// which every headless frame needs.
pub(crate) const WARM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Run `create` inside a `wgpu` validation error scope, and hand back what it captured.
///
/// This is the whole mechanism by which a refused shader becomes a value rather than a
/// panic. The scope is thread-local, so the warm-up thread and a frame compiling on
/// demand each capture their own without seeing the other's.
///
/// `pollster` resolves the pop, which blocks on nothing: `wgpu`'s own backend pops the
/// scope synchronously and returns an already-ready future, so this is the same "a
/// thread is not a runtime" position CLAUDE.md's stack table takes for the two awaits
/// in device creation.
fn captured<T>(device: &wgpu::Device, create: impl FnOnce() -> T) -> (T, Option<String>) {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let handle = create();
    let error = pollster::block_on(scope.pop());
    (handle, error.map(|error| error.to_string()))
}

/// The eight parsed WGSL modules, made together on first need.
///
/// Together because they are one artefact: the shaders this crate ships either all
/// parse on an adapter or the adapter cannot run this renderer, and there is no useful
/// state in between where some of a page draws.
struct Modules {
    rect: wgpu::ShaderModule,
    cover: wgpu::ShaderModule,
    image: wgpu::ShaderModule,
    shading: wgpu::ShaderModule,
    composite: wgpu::ShaderModule,
    reduce: wgpu::ShaderModule,
    blit: wgpu::ShaderModule,
    winding: wgpu::ShaderModule,
}

impl Modules {
    /// Parse every module, or name the first one this adapter refused.
    fn new(device: &wgpu::Device) -> Result<Self, PipelineProblem> {
        let module = |shader: &'static str, source: &str| {
            let (module, detail) = captured(device, || {
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(shader),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                })
            });
            match detail {
                Some(detail) => Err(PipelineProblem::Shader { shader, detail }),
                None => Ok(module),
            }
        };
        Ok(Self {
            rect: module("quorra rect", include_str!("shaders/rect.wgsl"))?,
            cover: module("quorra coverage", include_str!("shaders/coverage.wgsl"))?,
            image: module("quorra image", include_str!("shaders/image.wgsl"))?,
            shading: module("quorra shading", include_str!("shaders/shading.wgsl"))?,
            composite: module("quorra composite", include_str!("shaders/composite.wgsl"))?,
            reduce: module("quorra reduce", include_str!("shaders/reduce.wgsl"))?,
            blit: module("quorra blit", include_str!("shaders/blit.wgsl"))?,
            winding: module("quorra winding", include_str!("shaders/winding.wgsl"))?,
        })
    }
}

struct StoreState {
    layouts: Option<Layouts>,
    /// The parsed modules, or the refusal that stopped them — cached either way. A
    /// module a backend rejects is rejected identically every time it is asked for, so
    /// re-parsing per frame would cost the parse and answer nothing new; keeping the
    /// refusal is also what lets every later frame be refused in the same words.
    modules: Option<Result<Modules, PipelineProblem>>,
    pipelines: HashMap<(Kind, wgpu::TextureFormat), Arc<wgpu::RenderPipeline>>,
    warm_up: WarmUp,
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
            .field("warm_up", &state.warm_up)
            .finish_non_exhaustive()
    }
}

/// Records the warm-up's outcome and releases its waiters on **every** exit path,
/// including an unwind.
///
/// [`PipelineStore::wait_until_warm`] is a `Condvar` with exactly one notifier, so a
/// notifier that leaves by a route which does not notify leaves every waiter waiting
/// forever. That is not a hypothetical: a reserved keyword in `blit.wgsl` panicked this
/// thread inside `wgpu`, and the test binary then sat silent until it was killed — the
/// defect ADR 0042 exists to close. The error scopes above make the *known* panic a
/// value; this guard is what makes the promise hold for the ones that are not.
struct WarmUpGuard<'a> {
    store: &'a PipelineStore,
    /// What to record. Still `None` when the thread is unwinding, which is exactly
    /// [`WarmUp::Abandoned`] — the state with no reason to give, because there is none.
    outcome: Option<WarmUp>,
}

impl<'a> WarmUpGuard<'a> {
    fn new(store: &'a PipelineStore) -> Self {
        Self {
            store,
            outcome: None,
        }
    }

    /// Record `outcome` and release the waiters, by dropping.
    fn finish(mut self, outcome: WarmUp) {
        self.outcome = Some(outcome);
    }
}

impl Drop for WarmUpGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.store.lock();
        state.warm_up = self.outcome.take().unwrap_or(WarmUp::Abandoned);
        drop(state);
        self.store.warmed.notify_all();
    }
}

impl PipelineStore {
    /// A store with nothing compiled. Cheap by design: this runs inside
    /// `Device::headless`, whose contract is to return before pipelines exist.
    pub(crate) fn new(device: wgpu::Device) -> Arc<Self> {
        Arc::new(Self {
            device,
            state: Mutex::new(StoreState {
                layouts: None,
                modules: None,
                pipelines: HashMap::new(),
                warm_up: WarmUp::Running,
            }),
            warmed: Condvar::new(),
        })
    }

    /// Compile the warm set on a background thread, so device construction returns
    /// first. §2.1 of the brief also says a device must not *require* a background
    /// thread: if the host cannot spawn one, the warm set compiles inline instead —
    /// construction blocks for the compile, which is the documented cost of a host
    /// with no threads to give, and `warm_up` keeps meaning what it says.
    ///
    /// The handle goes to the [`Device`], which joins it when it is dropped. Nothing
    /// *waits* on it — completion is observed through `warm_up`/`wait_until_warm`, and
    /// a frame that arrives early compiles what it needs on the spot — but a thread
    /// inside the driver must not outlive the device it is compiling for (ADR 0018).
    ///
    /// [`Device`]: crate::device::Device
    pub(crate) fn spawn_warm_up(self: &Arc<Self>) -> Option<thread::JoinHandle<()>> {
        let store = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("quorra-warm-up".into())
            .spawn(move || store.warm_up_now());
        let Ok(handle) = spawned else {
            self.warm_up_now();
            return None;
        };
        Some(handle)
    }

    /// The warm-up itself: the two over-lanes a page of text needs, and the two passes
    /// a page with a group needs (§7 — the knockout variants, the reduction and the
    /// winding lane still compile on first use).
    ///
    /// **Why the compositor's two are here** (ADR 0040): a first frame with a group
    /// compiles [`Kind::Composite`] and [`Kind::Blit`] inside itself, and
    /// `Timings::phases` prices the pair at **0.75 ms at the minimum of 40 runs and
    /// 2.6 ms on the quietest one** on RADV — a third to a half of the 5 to 6 ms such a
    /// frame costs over its successors. Neither compile depends on the target's size,
    /// which is why no size hint could ever have moved them, and why the thread nobody
    /// blocks on is where they belong. A device that never composites pays for two
    /// pipelines it does not use, off the critical path, and reaches `is_warm` about
    /// 1.1 ms later.
    ///
    /// **A refusal is kept rather than retried** (ADR 0042), and the guard records one
    /// on every exit path including an unwind — a warm-up that ends without saying so
    /// leaves every waiter on a `Condvar` nobody will notify, which is how an invalid
    /// shader used to hang the process instead of failing it.
    fn warm_up_now(&self) {
        let guard = WarmUpGuard::new(self);
        let started = Instant::now();
        let compiled = self
            .get(Kind::RectOver, WARM_FORMAT)
            .and_then(|_| self.get(Kind::CoverOver, WARM_FORMAT))
            .and_then(|_| self.get(Kind::Composite, WARM_FORMAT))
            .and_then(|_| self.get(Kind::Blit, WARM_FORMAT));
        guard.finish(match compiled {
            Ok(_) => WarmUp::Warm(started.elapsed()),
            Err(reason) => WarmUp::Refused(reason),
        });
    }

    /// The pipeline for a lane and target format, compiling it now if the warm
    /// thread has not already. The second element is `Some(duration)` when this call
    /// did the compiling, so the frame that paid the one-off cost can name it in its
    /// `Timings::phases`.
    ///
    /// # Errors
    ///
    /// [`PipelineProblem`] when this adapter refuses one of the crate's shader modules
    /// or the pipeline built from them. Nothing is cached for a format that failed, so
    /// a later `get` for another format is unaffected — but a module that would not
    /// parse is remembered, because it will not parse next time either.
    pub(crate) fn get(
        &self,
        kind: Kind,
        format: wgpu::TextureFormat,
    ) -> Result<(Arc<wgpu::RenderPipeline>, Option<Duration>), PipelineProblem> {
        let mut state = self.lock();
        if let Some(pipeline) = state.pipelines.get(&(kind, format)) {
            return Ok((Arc::clone(pipeline), None));
        }
        let started = Instant::now();
        let pipeline = Arc::new(self.compile(&mut state, kind, format)?);
        state
            .pipelines
            .insert((kind, format), Arc::clone(&pipeline));
        Ok((pipeline, Some(started.elapsed())))
    }

    /// The bind-group layout for the globals uniform.
    pub(crate) fn globals_layout(&self) -> wgpu::BindGroupLayout {
        self.layout(|layouts| &layouts.globals)
    }

    /// The bind-group layout for the lane textures (atlas, scratch, soft mask).
    pub(crate) fn textures_layout(&self) -> wgpu::BindGroupLayout {
        self.layout(|layouts| &layouts.textures)
    }

    /// The image quad's bind-group layout.
    pub(crate) fn image_layout(&self) -> wgpu::BindGroupLayout {
        self.layout(|layouts| &layouts.image)
    }

    /// The shading quad's bind-group layout.
    pub(crate) fn shading_layout(&self) -> wgpu::BindGroupLayout {
        self.layout(|layouts| &layouts.shading)
    }

    /// The composite pass's bind-group layout.
    pub(crate) fn composite_layout(&self) -> wgpu::BindGroupLayout {
        self.layout(|layouts| &layouts.composite)
    }

    /// The reduce pass's bind-group layout.
    pub(crate) fn reduce_layout(&self) -> wgpu::BindGroupLayout {
        self.layout(|layouts| &layouts.reduce)
    }

    /// The winding sheet's globals layout, shared by both passes of the GPU lane.
    pub(crate) fn winding_layout(&self) -> wgpu::BindGroupLayout {
        self.layout(|layouts| &layouts.winding)
    }

    /// The blit pass's bind-group layout.
    pub(crate) fn blit_layout(&self) -> wgpu::BindGroupLayout {
        self.layout(|layouts| &layouts.blit)
    }

    /// The winding resolve's source layout: one sampled texture.
    pub(crate) fn sampled_layout(&self) -> wgpu::BindGroupLayout {
        self.layout(|layouts| &layouts.sampled)
    }

    /// One binding table, made with its siblings on first need.
    ///
    /// Infallible, and that is the point of `layouts.rs` being its own module: a
    /// layout is a description `wgpu` checks against nothing but itself, so no adapter
    /// can refuse one and no caller of these needs a `Result`.
    fn layout(
        &self,
        pick: impl FnOnce(&Layouts) -> &wgpu::BindGroupLayout,
    ) -> wgpu::BindGroupLayout {
        let mut state = self.lock();
        pick(Self::layouts(&self.device, &mut state)).clone()
    }

    /// Where the background warm-up has got to.
    pub(crate) fn warm_up(&self) -> WarmUp {
        self.lock().warm_up.clone()
    }

    /// How long the warm set took to compile, once it has.
    pub(crate) fn warm_duration(&self) -> Option<Duration> {
        let state = self.lock();
        match state.warm_up {
            WarmUp::Warm(duration) => Some(duration),
            _ => None,
        }
    }

    /// Block until the warm-up reports what became of it, and say what that was.
    /// Measurement support: startup numbers need a defined "fully warm" moment to be
    /// comparable.
    ///
    /// Returns for every outcome, including a warm-up that panicked
    /// ([`WarmUp::Abandoned`]) — see [`WarmUpGuard`].
    pub(crate) fn wait_until_warm(&self) -> WarmUp {
        let mut state = self.lock();
        while matches!(state.warm_up, WarmUp::Running) {
            state = self
                .warmed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.warm_up.clone()
    }

    fn lock(&self) -> MutexGuard<'_, StoreState> {
        // A poisoned lock means a compile panicked; every mutation under this lock is
        // an atomic insert or flag set, so the guarded data is still consistent and
        // continuing is sound.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The layouts alone, created on first need.
    fn layouts<'a>(device: &wgpu::Device, state: &'a mut StoreState) -> &'a Layouts {
        state.layouts.get_or_insert_with(|| Layouts::new(device))
    }

    /// Both halves of what a pipeline is built from, each created on first need.
    ///
    /// The destructuring is what lets one borrow of `state` produce two: `layouts` and
    /// `modules` are disjoint fields, so both can be filled and then both returned.
    fn base<'a>(
        device: &wgpu::Device,
        state: &'a mut StoreState,
    ) -> Result<(&'a Layouts, &'a Modules), PipelineProblem> {
        let StoreState {
            layouts, modules, ..
        } = state;
        let layouts = layouts.get_or_insert_with(|| Layouts::new(device));
        let modules = modules
            .get_or_insert_with(|| Modules::new(device))
            .as_ref()
            .map_err(Clone::clone)?;
        Ok((layouts, modules))
    }

    /// Build one pipeline, or name what this adapter refused.
    fn compile(
        &self,
        state: &mut StoreState,
        kind: Kind,
        format: wgpu::TextureFormat,
    ) -> Result<wgpu::RenderPipeline, PipelineProblem> {
        let (layouts, modules) = Self::base(&self.device, state)?;
        let spec = Spec::of(kind, layouts, modules);
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
        let (pipeline, detail) = captured(&self.device, || {
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
        });
        match detail {
            Some(detail) => Err(PipelineProblem::Pipeline {
                pipeline: spec.label,
                format,
                detail,
            }),
            None => Ok(pipeline),
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)] // test-file policy: a fixture that cannot run must fail loudly
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::{Kind, PipelineStore, WARM_FORMAT, WarmUpGuard};
    use crate::device::Device;
    use crate::error::PipelineProblem;
    use crate::startup::{Options, WarmUp};

    /// The software adapter, as everywhere in this crate's tests.
    fn device() -> Device {
        Device::headless(&Options {
            adapter: Some("llvmpipe".into()),
            ..Options::default()
        })
        .expect("llvmpipe is present wherever this suite runs")
    }

    /// **How long a wait is allowed to be before it counts as never ending.** Every
    /// test here that could hang runs its wait on a thread and gives up after this, so
    /// a regression fails the suite instead of wedging it — which is the trap the
    /// defect these tests guard against actually sprang.
    const PATIENCE: Duration = Duration::from_secs(30);

    /// Call `wait` on a thread and insist it returns. Panics with `what` if it does
    /// not, leaving the waiting thread behind: it is blocked on a `Condvar` and holds
    /// nothing, so the process still exits.
    fn within_patience<T: Send + 'static>(what: &str, wait: impl FnOnce() -> T + Send + 'static) {
        let (done, waited) = mpsc::channel();
        thread::spawn(move || {
            drop(wait());
            let _ = done.send(());
        });
        assert!(
            waited.recv_timeout(PATIENCE).is_ok(),
            "{what} did not return within {PATIENCE:?}"
        );
    }

    /// The defect ADR 0042 closes, stated as the property that failed: a warm-up that
    /// ends by panicking must still release everything waiting on it. Before the guard,
    /// this test hung — which is the whole reason the wait is bounded.
    #[test]
    fn a_warm_up_that_panics_still_releases_its_waiters() {
        let device = device();
        let (gpu, _) = device.wgpu();
        let store = PipelineStore::new(gpu.clone());
        let panicking = {
            let store = Arc::clone(&store);
            thread::spawn(move || {
                let _guard = WarmUpGuard::new(&store);
                panic!("a driver error wgpu reports by panicking rather than by an error scope");
            })
        };
        assert!(panicking.join().is_err(), "the thread panicked as staged");
        within_patience("wait_until_warm after a panicking warm-up", {
            let store = Arc::clone(&store);
            move || store.wait_until_warm()
        });
        assert_eq!(store.warm_up(), WarmUp::Abandoned);
        assert!(store.warm_duration().is_none());
    }

    /// A store whose warm-up has not run yet keeps `wait_until_warm` waiting — the
    /// other half of the property above, so that "it returns" is not passing by
    /// returning always.
    #[test]
    fn a_running_warm_up_is_reported_as_running() {
        let device = device();
        let (gpu, _) = device.wgpu();
        let store = PipelineStore::new(gpu.clone());
        assert_eq!(store.warm_up(), WarmUp::Running);
    }

    /// A pipeline this adapter cannot build is an `Err` naming it, not a panic on
    /// whichever thread asked (§5: refused, never survived).
    ///
    /// `Rgba8Snorm` is the instrument: WebGPU gives it no `RENDER_ATTACHMENT` usage, so
    /// a colour target in that format is a validation error every backend agrees on,
    /// reached without shipping a shader that does not parse.
    #[test]
    fn a_pipeline_the_adapter_refuses_is_an_error_and_not_a_panic() {
        let device = device();
        let (gpu, _) = device.wgpu();
        let store = PipelineStore::new(gpu.clone());
        let refused = store.get(Kind::Blit, wgpu::TextureFormat::Rgba8Snorm);
        match refused {
            Err(PipelineProblem::Pipeline {
                pipeline, format, ..
            }) => {
                assert_eq!(pipeline, "quorra blit");
                assert_eq!(format, wgpu::TextureFormat::Rgba8Snorm);
            }
            other => panic!("expected a named pipeline refusal, got {other:?}"),
        }
        // Nothing was cached for the format that failed, and the store still works:
        // one refused format must not take the device with it.
        store
            .get(Kind::Blit, WARM_FORMAT)
            .expect("the blit pipeline builds for the format the frame actually uses");
    }

    /// The refusal above must reach a caller in the words of the error, not only as a
    /// variant — a person reading a log has to be able to attribute it.
    #[test]
    fn a_refusal_names_the_pipeline_and_the_format() {
        let device = device();
        let (gpu, _) = device.wgpu();
        let store = PipelineStore::new(gpu.clone());
        let Err(reason) = store.get(Kind::Composite, wgpu::TextureFormat::Rgba8Snorm) else {
            panic!("Rgba8Snorm is not a renderable format");
        };
        let message = reason.to_string();
        assert!(message.contains("quorra composite"), "{message}");
        assert!(message.contains("Rgba8Snorm"), "{message}");
    }
}
