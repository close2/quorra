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
//! - **Few.** The warm set is the two lanes a page of text needs — the analytic
//!   rectangle (`Kind::RectOver`) and the coverage quad (`Kind::CoverOver`) — plus
//!   the compositor's `Kind::Composite` and `Kind::Blit`, which a first frame with a
//!   group otherwise compiles inside itself (ADR 0040). Everything else — knockout
//!   variants, the image and shading quads (ADR 0011) — compiles on first use. Each
//!   kind is instantiated per target format, and a device constructed for a surface
//!   warms the presenting lanes in the surface's negotiated format too (ADR 0043) —
//!   including `Kind::Present`, the one pass a detached
//!   [`Presenter`](crate::present::Presenter) draws with, so that detaching one
//!   compiles nothing (ADR 0056).
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
//! inside a validation error scope (`captured`), a captured failure becomes a
//! [`PipelineProblem`], and the store's `get` is fallible so the frame that needed
//! the pipeline is refused by name (ADR 0042). What the warm-up thread ends with is
//! recorded in [`WarmUp`] on **every** exit path, so its `wait_until_warm`
//! always returns.
//!
//! # The module's five files, and what each one's one thing is
//!
//! Fallibility is the seam three of them are cut along: `layouts.rs` is the half of a
//! pipeline no adapter can refuse, `spec.rs` is what each pipeline `Kind` *is*, and
//! this file is the store. `warm.rs` is cut along a different one — it is a state
//! machine with a thread of its own — and `function.rs` along a third, the key. rustdoc
//! inlines a re-export from a private module (ADR 0051 §1), so this table is the only
//! place the structure survives into the documentation:
//!
//! | Module | Its one thing |
//! |---|---|
//! | `pipeline.rs` | the store: one lock, one map, laziness, and the compile that fills it |
//! | `pipeline/layouts.rs` | the binding tables every pipeline is built against — the half that cannot be refused |
//! | `pipeline/spec.rs` | what each `Kind` *is*: its shader, entry points, vertex layout and blend state |
//! | `pipeline/warm.rs` | the warm-up: which pipelines are compiled before anyone asks, and the state machine that reports what became of that |
//! | `pipeline/function.rs` | ADR 0053's generated shaders, keyed by a program's content hash rather than by a `Kind` |
//!
//! The store knows nothing about the warm-up beyond one field: `warm.rs` asks
//! `PipelineStore::get` for the pipelines it wants like any other caller, which is why
//! **a change to what a device warms cannot change what a frame compiles.**
//!
//! # Shaders are code, and this project's rules apply to them
//!
//! WGSL lives in `src/shaders/`, and every function implementing a normative
//! requirement carries its clause number in a comment — a WGSL blend function is not
//! exempt from CLAUDE.md principle 5 just because it is not Rust. A shader whose
//! invariants are not stated beside it is write-only code (principle 4); each shader
//! states its coverage definition and its determinism argument inline.

mod function;
mod layouts;
mod spec;
mod warm;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::error::PipelineProblem;
use crate::function::ProgramHash;
use crate::shaders;
use crate::startup::WarmUp;
use function::FunctionKey;
use layouts::Layouts;
use spec::Spec;

pub(crate) use spec::{Kind, Style};

/// The format the warm-up thread compiles for: the readback and host-texture format,
/// which every headless frame needs.
pub(crate) const WARM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Run `create` inside a `wgpu` validation error scope, and hand back what it captured.
///
/// This is the whole mechanism by which a refused shader becomes a value rather than a
/// panic. The scope is thread-local, so the warm-up thread and a frame compiling on
/// demand each capture their own without seeing the other's — and so does a
/// [`Presenter`](crate::present::Presenter) binding a layer texture on a third thread,
/// which is the other creation in this crate whose failure must be a value (ADR 0056).
///
/// `pollster` resolves the pop, which blocks on nothing: `wgpu`'s own backend pops the
/// scope synchronously and returns an already-ready future, so this is the same "a
/// thread is not a runtime" position CLAUDE.md's stack table takes for the two awaits
/// in device creation.
pub(crate) fn captured<T>(
    device: &wgpu::Device,
    create: impl FnOnce() -> T,
) -> (T, Option<String>) {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let handle = create();
    let error = pollster::block_on(scope.pop());
    (handle, error.map(|error| error.to_string()))
}

/// The nine parsed WGSL modules, made together on first need.
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
    present: wgpu::ShaderModule,
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
            rect: module("quorra rect", shaders::RECT)?,
            cover: module("quorra coverage", shaders::COVERAGE)?,
            image: module("quorra image", shaders::IMAGE)?,
            shading: module("quorra shading", shaders::SHADING)?,
            composite: module("quorra composite", shaders::COMPOSITE)?,
            reduce: module("quorra reduce", shaders::REDUCE)?,
            blit: module("quorra blit", shaders::BLIT)?,
            present: module("quorra present", shaders::PRESENT)?,
            winding: module("quorra winding", shaders::WINDING)?,
        })
    }
}

/// The store's mutable half, behind its one lock.
///
/// The three `function_*` maps are the generated shaders of ADR 0053, and they are a
/// second table rather than more entries in the first because their key is a program's
/// content hash rather than a [`Kind`]: what is compiled is a function of what a caller
/// uploaded, so it cannot be enumerated at construction and is never in the warm set.
/// `pipeline/function.rs` owns every operation on them.
struct StoreState {
    layouts: Option<Layouts>,
    /// The parsed modules, or the refusal that stopped them — cached either way. A
    /// module a backend rejects is rejected identically every time it is asked for, so
    /// re-parsing per frame would cost the parse and answer nothing new; keeping the
    /// refusal is also what lets every later frame be refused in the same words.
    modules: Option<Result<Modules, PipelineProblem>>,
    pipelines: HashMap<(Kind, wgpu::TextureFormat), Arc<wgpu::RenderPipeline>>,
    /// One parsed module per generated shader, shared by that shader's three styles.
    function_modules: HashMap<ProgramHash, wgpu::ShaderModule>,
    /// Which program each generated shader came from, so that a released program can find
    /// every shader it generated.
    function_shaders: HashMap<ProgramHash, ProgramHash>,
    function_pipelines: HashMap<FunctionKey, Arc<wgpu::RenderPipeline>>,
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
    /// Released when `state.warm_up` stops being [`WarmUp::Running`]. **Every line that
    /// waits on it or notifies it is in `warm.rs`**, which is what makes "exactly one
    /// notifier, and it notifies on every exit path" a property of one file rather than
    /// a habit spread over two.
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
                function_modules: HashMap::new(),
                function_shaders: HashMap::new(),
                function_pipelines: HashMap::new(),
                warm_up: WarmUp::Running,
            }),
            warmed: Condvar::new(),
        })
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

    /// The function quad's bind-group layout (ADR 0053).
    pub(crate) fn function_layout(&self) -> wgpu::BindGroupLayout {
        self.layout(|layouts| &layouts.function)
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

    /// The present pass's bind-group layout (ADR 0056).
    pub(crate) fn present_layout(&self) -> wgpu::BindGroupLayout {
        self.layout(|layouts| &layouts.present)
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
