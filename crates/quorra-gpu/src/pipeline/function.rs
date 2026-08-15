//! One pipeline per §7.10.5 program, cached by what the program *is* (ADR 0053).
//!
//! The rest of the store's pipelines are a fixed table: seventeen [`Kind`](super::Kind)s
//! over the target formats a device meets. This one is not a table at all — the shader's
//! text is a function of a program the caller uploaded — so it needs a key of its own, and
//! ADR 0053 says which:
//!
//! > worth building, for type 4 only, and only as a generated shader cached by the
//! > program's hash.
//!
//! # The key, and why it is the shader's hash rather than the program's
//!
//! [`Analysis::shader_hash`] mixes the `Range`'s component count into
//! [`Analysis::program_hash`], because one program under a `Gray` range and the same
//! program under an `Rgb` one emit different functions. The upload table keys on the
//! program's hash and this keys on the shader's: two questions, two hashes, and the pair
//! is what makes "one generated shader per distinct program" true for the thing that is
//! actually compiled.
//!
//! The full key is `(shader hash, style, target format)`. The style is ADR 0010's three —
//! a knockout element compiles the erase and the add as well as the over — and the format
//! is there for the same reason every other pipeline carries it (ADR 0043: a presenting
//! surface negotiates `Bgra8Unorm`).
//!
//! # Nothing here is on the launch path
//!
//! `doc/PLAN.md` §1.8: nothing on the launch path waits for warmth. A generated pipeline
//! **cannot** be in the warm set — the program does not exist when the device is
//! constructed — so it compiles on the first frame that draws it, at the 6.3 ms
//! `doc/spike-function-paint.md` §3 measured cold for a 482-instruction witness, and that
//! frame names the cost in its own `Timings::phases` like every other first-use compile.
//! A caller that wants the compile off its first frame uploads the program early: the
//! upload is what admits it, and admission is the expensive question a page cannot pay
//! twice.
//!
//! # A compile that fails is a refusal, never a hang
//!
//! ADR 0042's rule, applied to text this tree did not write: the compile runs inside a
//! validation error scope, a captured failure becomes a
//! [`PipelineProblem::GeneratedShader`] or [`PipelineProblem::GeneratedPipeline`] naming
//! the program's hash and carrying `wgpu`'s message — span included, which for generated
//! source is the only way to say *where* — and the frame that needed it is refused by
//! name. Nothing is cached for a program that failed, so a defect in the generator is
//! reported on every frame that reaches it rather than once.

use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::time::{Duration, Instant};

use quorra_scene::FnRange;

use super::spec::{Spec, Style};
use super::{Layouts, PipelineStore, StoreState, captured};
use crate::error::PipelineProblem;
use crate::function::{Analysis, ProgramHash, generate};

/// The fixed half of every generated shader: the quad, the coverage, the clip and the
/// soft mask. Appended to what [`generate`] emits, which supplies the operator library and
/// `quorra_function_evaluate`.
const LANE: &str = include_str!("../shaders/function_lane.wgsl");

/// What one compiled function pipeline is keyed by.
///
/// Ordered as it is read: which shader, which of ADR 0010's styles, which target format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct FunctionKey {
    pub(crate) shader: ProgramHash,
    pub(crate) style: Style,
    pub(crate) format: wgpu::TextureFormat,
}

impl PipelineStore {
    /// The pipeline that paints one admitted program under one `Range`, compiling it on
    /// first use.
    ///
    /// The second element is `Some(duration)` when this call did the compiling, so the
    /// frame that paid the one-off cost can name it in its `Timings::phases` — the same
    /// contract [`PipelineStore::get`] has, because a first-use compile is the same event
    /// whichever table it lands in.
    ///
    /// # Errors
    ///
    /// [`PipelineProblem::GeneratedShader`] when the generator refuses the pairing of this
    /// program with this `Range`, or when the adapter refuses the module it emitted;
    /// [`PipelineProblem::GeneratedPipeline`] when the pipeline built from a module that
    /// parsed is itself refused.
    pub(crate) fn function_pipeline(
        &self,
        analysis: &Analysis,
        range: FnRange,
        style: Style,
        format: wgpu::TextureFormat,
    ) -> Result<(Arc<wgpu::RenderPipeline>, Option<Duration>), PipelineProblem> {
        let key = FunctionKey {
            shader: analysis.shader_hash(range),
            style,
            format,
        };
        let mut state = self.lock();
        if let Some(pipeline) = state.function_pipelines.get(&key) {
            return Ok((Arc::clone(pipeline), None));
        }
        let started = Instant::now();
        let pipeline = Arc::new(self.compile_function(&mut state, key, analysis, range)?);
        state.function_pipelines.insert(key, Arc::clone(&pipeline));
        Ok((pipeline, Some(started.elapsed())))
    }

    /// Drop every pipeline and module a program's hash keys.
    ///
    /// Called when the last resident program with that hash is released: a compiled
    /// pipeline the caller can no longer name is GPU memory nothing will ever free, which
    /// is the leak an identifier without a release path would have been.
    pub(crate) fn forget_program(&self, program: ProgramHash) {
        let mut state = self.lock();
        // The shader hash is the program's mixed with a component count, so it is not the
        // program's; the entries carry the shader hash, and the module map is keyed by it
        // too. Both are walked, because a `Gray` and an `Rgb` placement of one program are
        // two shaders and only the program knows they are siblings.
        let shaders: Vec<ProgramHash> = state
            .function_shaders
            .iter()
            .filter(|(_, of)| **of == program)
            .map(|(shader, _)| *shader)
            .collect();
        state
            .function_pipelines
            .retain(|key, _| !shaders.contains(&key.shader));
        state
            .function_modules
            .retain(|shader, _| !shaders.contains(shader));
        state.function_shaders.retain(|_, of| *of != program);
    }

    /// How many generated pipelines are compiled.
    ///
    /// A **count of distinct keys**, which is what CLAUDE.md's second instrumentation rule
    /// asks for: a hit rate would be a statement about the lookups a page made, and what
    /// costs 6.3 ms each is the entries.
    #[cfg(test)]
    fn function_pipelines_compiled(&self) -> usize {
        self.lock().function_pipelines.len()
    }

    /// How many generated modules are parsed, which is the other thing a miss costs.
    #[cfg(test)]
    fn function_modules_parsed(&self) -> usize {
        self.lock().function_modules.len()
    }

    /// Generate, parse and build — the three steps a miss costs, each refusable by name.
    fn compile_function(
        &self,
        state: &mut StoreState,
        key: FunctionKey,
        analysis: &Analysis,
        range: FnRange,
    ) -> Result<wgpu::RenderPipeline, PipelineProblem> {
        // Destructured for the same reason `PipelineStore::base` destructures: the layouts
        // and the modules are disjoint fields, so one borrow of the state can produce two
        // and the pipeline can be built from both at once.
        let StoreState {
            layouts,
            function_modules,
            function_shaders,
            ..
        } = state;
        let layouts = layouts.get_or_insert_with(|| Layouts::new(&self.device));
        let module = match function_modules.entry(key.shader) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let module = Self::function_module(&self.device, key.shader, analysis, range)?;
                // Which program this shader belongs to, so that releasing the program can
                // find every shader it generated — a `Gray` and an `Rgb` placement of one
                // program are two shaders, and only the program knows they are siblings.
                function_shaders.insert(key.shader, analysis.program_hash());
                entry.insert(module)
            }
        };
        let spec = Spec::function(module, layouts, key.style);
        build(&self.device, &spec, key)
    }

    /// The generated WGSL for one program under one `Range`, parsed by this adapter.
    fn function_module(
        device: &wgpu::Device,
        shader: ProgramHash,
        analysis: &Analysis,
        range: FnRange,
    ) -> Result<wgpu::ShaderModule, PipelineProblem> {
        let generated = generate(analysis, range).map_err(|reason| {
            // Unreachable from a frame: the encoder asks `Analysis::admits` before it
            // places the quad, so a `Range` this program cannot fill is already a refused
            // frame. Spelled out rather than unwrapped, because the generator's one
            // refusal is a fact about the pairing and this is the honest place to say so.
            PipelineProblem::GeneratedShader {
                program: shader,
                detail: reason.to_string(),
            }
        })?;
        let source = format!("{}\n{LANE}", generated.module());
        let (module, detail) = captured(device, || {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("quorra function"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        });
        match detail {
            Some(detail) => Err(PipelineProblem::GeneratedShader {
                program: shader,
                detail,
            }),
            None => Ok(module),
        }
    }
}

/// The pipeline itself, inside an error scope, refused by name.
///
/// Separate from [`PipelineStore::compile`] rather than shared with it: that one reads a
/// `Kind` out of a fixed table and labels its refusal with a `&'static str`, and this one
/// has a hash where that label would be. What they would share is the descriptor, which is
/// [`Spec`]'s job and is already shared.
fn build(
    device: &wgpu::Device,
    spec: &Spec<'_>,
    key: FunctionKey,
) -> Result<wgpu::RenderPipeline, PipelineProblem> {
    let (pipeline, detail) = captured(device, || {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(spec.label),
            layout: Some(spec.layout),
            vertex: wgpu::VertexState {
                module: spec.shader,
                entry_point: Some(spec.vertex),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: spec.shader,
                entry_point: Some(spec.entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: key.format,
                    blend: spec.blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    });
    match detail {
        Some(detail) => Err(PipelineProblem::GeneratedPipeline {
            program: key.shader,
            format: key.format,
            detail,
        }),
        None => Ok(pipeline),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)] // test-file policy: a fixture that cannot run must fail loudly
mod tests {
    use std::sync::Arc;

    use quorra_scene::{FnOp, FnRange};

    use super::Style;
    use crate::device::Device;
    use crate::error::PipelineProblem;
    use crate::function::analyse;
    use crate::pipeline::{PipelineStore, WARM_FORMAT};
    use crate::startup::Options;

    /// The two-input program `x y add`: admitted, and the shortest thing that is a real
    /// §7.10.5 function rather than a constant.
    const ADD: [FnOp; 1] = [FnOp::Add];

    /// A store of its own, on the software adapter as everywhere in this crate's tests.
    /// The device is returned with it because the store borrows its `wgpu::Device`.
    fn store() -> (Device, Arc<PipelineStore>) {
        let device = Device::headless(&Options {
            adapter: Some("llvmpipe".into()),
            ..Options::default()
        })
        .expect("llvmpipe is present wherever this suite runs");
        let (gpu, _) = device.wgpu();
        let store = PipelineStore::new(gpu.clone());
        (device, store)
    }

    /// **The second ask is answered rather than compiled**, which is the whole of what
    /// ADR 0053's "cached by the program's hash" buys: 6.3 ms cold for the caller's larger
    /// witness, against a page that may draw one shading a thousand times.
    #[test]
    fn one_program_under_one_range_compiles_one_pipeline() {
        let (_device, store) = store();
        let analysis = analyse(&ADD).expect("`add` is admitted");
        let range = FnRange::Gray([0.0, 1.0]);

        let (first, compiled) = store
            .function_pipeline(&analysis, range, Style::Over, WARM_FORMAT)
            .expect("the generated shader compiles on this adapter");
        assert!(compiled.is_some(), "the first ask does the compiling");
        let (second, again) = store
            .function_pipeline(&analysis, range, Style::Over, WARM_FORMAT)
            .expect("the second ask is answered");
        assert!(again.is_none(), "the second ask compiles nothing");
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(store.function_pipelines_compiled(), 1);
        assert_eq!(store.function_modules_parsed(), 1);
    }

    /// A knockout element wants the erase/add pair as well as the over, and all three come
    /// from **one** parsed module: the styles differ in their entry point and their blend
    /// state, which is a pipeline's business rather than a shader's.
    #[test]
    fn the_three_styles_share_one_generated_module() {
        let (_device, store) = store();
        let analysis = analyse(&ADD).expect("`add` is admitted");
        let range = FnRange::Gray([0.0, 1.0]);
        for style in [Style::Over, Style::Erase, Style::Add] {
            store
                .function_pipeline(&analysis, range, style, WARM_FORMAT)
                .expect("every style of the generated lane compiles");
        }
        assert_eq!(store.function_pipelines_compiled(), 3);
        assert_eq!(
            store.function_modules_parsed(),
            1,
            "one shader, three pipelines — a module per style would be three parses"
        );
    }

    /// Two programs of different component counts are **two** shaders, and their keys
    /// differ in the component count as well as in the instructions.
    ///
    /// One program cannot supply both counts — ISO 32000-2 §7.10.5.3 makes the output count
    /// an equality — so the component count in the key is redundant with the program today.
    /// It stays because it is what makes the key correct independently of who enforces the
    /// count, and the assertion below is on the key rather than on two compiled shaders for
    /// that reason.
    #[test]
    fn two_component_counts_are_two_shaders() {
        let (_device, store) = store();
        let one = analyse(&ADD).expect("`add` leaves one value");
        // `x y 1 index 1 index mul`: three values left, so the three-component range is
        // the one §7.10.5.3 admits for it.
        let three = analyse(&[
            FnOp::PushInt(1),
            FnOp::Index,
            FnOp::PushInt(1),
            FnOp::Index,
            FnOp::Mul,
        ])
        .expect("the program leaves three values");
        let gray = FnRange::Gray([0.0, 1.0]);
        let rgb = FnRange::Rgb([[0.0, 1.0]; 3]);
        assert_ne!(one.shader_hash(gray), one.shader_hash(rgb));

        store
            .function_pipeline(&one, gray, Style::Over, WARM_FORMAT)
            .expect("the one-component shader compiles");
        store
            .function_pipeline(&three, rgb, Style::Over, WARM_FORMAT)
            .expect("the three-component shader compiles");
        assert_eq!(store.function_modules_parsed(), 2);
    }

    /// Releasing the last program with a hash drops what it compiled: a pipeline nothing
    /// can name again is GPU memory nothing will ever free — the leak an identifier
    /// without a release path would have been.
    #[test]
    fn forgetting_a_program_drops_its_pipelines() {
        let (_device, store) = store();
        let analysis = analyse(&ADD).expect("`add` is admitted");
        let range = FnRange::Gray([0.0, 1.0]);
        store
            .function_pipeline(&analysis, range, Style::Over, WARM_FORMAT)
            .expect("the generated shader compiles");
        assert_eq!(store.function_pipelines_compiled(), 1);

        store.forget_program(analysis.program_hash());
        assert_eq!(store.function_pipelines_compiled(), 0);
        assert_eq!(store.function_modules_parsed(), 0);
        // The same instructions uploaded again reach the same key and compile again, which
        // is why dropping is safe rather than merely tidy.
        let (_, compiled) = store
            .function_pipeline(&analysis, range, Style::Over, WARM_FORMAT)
            .expect("the generated shader compiles again");
        assert!(compiled.is_some());
    }

    /// A target format no adapter can render to is refused **by name**, naming the program
    /// where the fixed table would name a `&'static str` — ADR 0042's rule applied to text
    /// this tree did not write. `Rgba8Snorm` is the instrument `pipeline.rs`'s own tests
    /// use: WebGPU gives it no `RENDER_ATTACHMENT` usage.
    #[test]
    fn a_refused_generated_pipeline_names_its_program() {
        let (_device, store) = store();
        let analysis = analyse(&ADD).expect("`add` is admitted");
        let range = FnRange::Gray([0.0, 1.0]);
        let refused = store.function_pipeline(
            &analysis,
            range,
            Style::Over,
            wgpu::TextureFormat::Rgba8Snorm,
        );
        match refused {
            Err(PipelineProblem::GeneratedPipeline {
                program, format, ..
            }) => {
                assert_eq!(program, analysis.shader_hash(range));
                assert_eq!(format, wgpu::TextureFormat::Rgba8Snorm);
            }
            other => panic!("expected a named generated-pipeline refusal, got {other:?}"),
        }
        assert_eq!(
            store.function_pipelines_compiled(),
            0,
            "nothing is cached for a format that failed"
        );
        // The module parsed, so it is kept: what failed is the pipeline, and the next
        // format asked for must not pay the parse again.
        assert_eq!(store.function_modules_parsed(), 1);
    }
}
