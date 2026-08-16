//! What this adapter would not build.
//!
//! The one refusal in this crate that outlives the call that raised it, which is why it
//! is a type of its own and not a variant's fields. `PipelineStore` keeps one so that a
//! module this backend rejects reaches *every* later frame with the same words, the
//! background warm-up thread carries it across a thread boundary to get it there, and
//! [`WarmUp::Refused`](crate::startup::WarmUp::Refused) hands it to a caller who has not
//! asked for a frame at all (ADR 0042). It reaches a frame as
//! [`RenderError::PipelineUnavailable`](crate::error::RenderError::PipelineUnavailable).

use thiserror::Error;

/// Why a shader module or a render pipeline could not be built on this adapter.
///
/// Its own type rather than a [`RenderError`](crate::error::RenderError) variant's
/// fields, and deliberately
/// `Clone` and free of `wgpu`'s own error types: the pipeline store keeps one of these
/// so that a refusal reaches *every* later frame with the same words, and the
/// background warm-up thread has to carry it across a thread boundary to do that.
///
/// A shader that does not parse is this crate's own defect and not something a scene
/// can provoke — but the same refusal is how a backend that will not accept one of our
/// shaders, or a target format it cannot render to, arrives, and neither of those may
/// be a panic on a thread nobody is listening to (§5 of the brief).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PipelineProblem {
    /// A WGSL module in `src/shaders/` was refused: a parse or validation failure that
    /// this adapter's backend reported.
    #[error("shader module '{shader}' was refused: {detail}")]
    Shader {
        /// The module's label, as `src/pipeline.rs` names it.
        shader: &'static str,
        /// What `wgpu` said, including the source span when it had one.
        detail: String,
    },
    /// A *generated* §7.10.5 shader was refused (ADR 0053, ADR 0042).
    ///
    /// Its own variant rather than [`PipelineProblem::Shader`] because there is no
    /// `&'static str` to name it by: the module's text is a function of a program the
    /// caller uploaded, so what identifies it is that program's content hash. The detail
    /// carries `wgpu`'s message, span included, which for generated text is the only way
    /// to say *where* — the source it points into is not in this tree.
    #[error("the generated shader for §7.10.5 program {program} was refused: {detail}")]
    GeneratedShader {
        /// The program's content hash, as [`GeneratedShader::hash`] gives it.
        ///
        /// [`GeneratedShader::hash`]: crate::function::GeneratedShader::hash
        program: crate::function::ProgramHash,
        /// What `wgpu` said, including the source span when it had one.
        detail: String,
    },
    /// A pipeline built from a generated §7.10.5 shader that parsed was itself refused.
    #[error("the pipeline for §7.10.5 program {program} for {format:?} was refused: {detail}")]
    GeneratedPipeline {
        /// The program's content hash.
        program: crate::function::ProgramHash,
        /// The colour target format it was asked for.
        format: wgpu::TextureFormat,
        /// What `wgpu` said.
        detail: String,
    },
    /// A render pipeline built from modules that parsed was itself refused — an entry
    /// point, a vertex layout or a colour target this adapter will not accept.
    #[error("pipeline '{pipeline}' for {format:?} was refused: {detail}")]
    Pipeline {
        /// The pipeline's label.
        pipeline: &'static str,
        /// The colour target format it was asked for.
        format: wgpu::TextureFormat,
        /// What `wgpu` said.
        detail: String,
    },
}
