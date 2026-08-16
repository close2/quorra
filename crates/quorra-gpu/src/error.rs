//! Every way this crate refuses, in two typed enums.
//!
//! §5 of the brief: a failure is an `Err` that names what overflowed or did not hold,
//! so the caller can act on it — its window falls back to the CPU backend, and a
//! person reading a log can attribute the refusal without reproducing it. There is no
//! catch-all variant in either enum, for the same reason `ReportKind` has no `Other`:
//! a variant that names nothing makes "how often does this happen?" unanswerable.
//!
//! The split follows the two phases of a device's life: [`DeviceError`] is
//! construction (§2.1), [`RenderError`] is a frame (§2.4) — and a present, which is a
//! frame's last step happening somewhere else (ADR 0056). A report — the frame *was*
//! drawn, something in it is not as asked — is deliberately neither; that is
//! [`crate::report`].
//!
//! The other types here are vocabularies those two carry: what an upload violated
//! ([`ResourceProblem`], [`FunctionProblem`]), what an adapter refused
//! ([`PipelineProblem`]), what a surface reported ([`SurfaceProblem`]), and what a
//! layer did not satisfy ([`LayerProblem`]). Each is an enum for the reason
//! [`crate::report::ReportKind`] is one: "how often does this happen?" must stay
//! answerable, which a catch-all variant would end.

use thiserror::Error;

use crate::frame::CoverageSheet;

/// Why a device could not be constructed. Every variant names what was unavailable.
#[derive(Debug, Error)]
pub enum DeviceError {
    /// No adapter matched the request. Carries every adapter that was available so
    /// the caller (or the person reading the error) can see what would have matched.
    #[error("no adapter matched {requested:?}; adapters present: {available:?}")]
    NoAdapter {
        /// The `Options::adapter` filter that was applied, if any.
        requested: Option<String>,
        /// The names of every adapter enumeration found.
        available: Vec<String>,
    },
    /// The adapter refused to yield a device.
    #[error("device creation failed on adapter '{adapter}': {source}")]
    DeviceCreation {
        /// The adapter that refused.
        adapter: String,
        /// wgpu's reason.
        source: wgpu::RequestDeviceError,
    },
    /// The window handle could not become a surface.
    #[error("surface creation failed: {source}")]
    SurfaceCreation {
        /// wgpu's reason.
        source: wgpu::CreateSurfaceError,
    },
    /// The chosen adapter cannot present to the given surface at all.
    #[error("adapter '{adapter}' offers no format for this surface")]
    SurfaceUnsupported {
        /// The adapter that cannot present.
        adapter: String,
    },
    /// An upload would push resident resources past the stated budget
    /// (`Options::max_resource_bytes`). Nothing was stored.
    #[error(
        "uploading would hold {needed} resource bytes ({in_use} already resident), over the stated budget of {budget}"
    )]
    ResourceBudgetExceeded {
        /// Bytes that would be resident after the upload.
        needed: u64,
        /// Bytes resident before it.
        in_use: u64,
        /// The configured budget.
        budget: u64,
    },
    /// An upload's content violated its contract (§4.7 of the brief: refused loudly,
    /// never repaired).
    #[error("upload refused: {reason}")]
    InvalidResource {
        /// What exactly was wrong.
        reason: ResourceProblem,
    },
    /// A §7.10.5 program this device will not execute (ADR 0053). Its own variant
    /// rather than a [`ResourceProblem`] because the questions are of a different kind:
    /// a ramp is refused for what its numbers *are*, a program for what it would *do*,
    /// and the answer reaches the caller before it has built a scene at all.
    #[error("function program refused: {reason}")]
    InvalidFunction {
        /// Which of the upload's three questions was answered no, and by what.
        reason: FunctionProblem,
    },
    /// This device has issued every resource identifier it has. Nothing was stored.
    ///
    /// The identifier space is a `u32` shared by the four resource families and an id
    /// is **never** reused, so a store that has issued `u32::MAX` of them cannot admit
    /// another. Refusing is the only honest answer: reusing an id would silently replace
    /// whatever still held it, and a retained encode naming that id would draw the wrong
    /// resource with every generation counter still agreeing that nothing had moved
    /// (ADR 0048's key, ADR 0050's audit of it). Releasing resources returns bytes to
    /// the budget but never returns identifiers.
    #[error("this device has issued all {limit} resource identifiers; none can be reused")]
    ResourceIdsExhausted {
        /// The size of the identifier space, which is also how many were issued.
        limit: u32,
    },
    /// A release of a resource this device never issued or already released. An error
    /// rather than a no-op: a double release is a caller bug, and hiding it would
    /// hide the defect.
    #[error("resource {id:?} is not resident on this device")]
    UnknownResource {
        /// The identifier that was presented.
        id: quorra_scene::ResourceId,
    },
}

/// What exactly an upload violated. Enumerated for the same reason `ReportKind` is:
/// "how often does this happen?" must stay answerable.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum ResourceProblem {
    /// An outline with no segments at all.
    #[error("outline has no segments")]
    OutlineEmpty,
    /// An outline whose first segment is not a `MoveTo` — no current point exists for
    /// anything else to draw from (ISO 32000-2 §8.5.2's path construction starts
    /// every path with `m`).
    #[error("outline does not start with MoveTo")]
    OutlineMissingMoveTo,
    /// An outline containing a NaN or infinite coordinate.
    #[error("outline has a non-finite coordinate")]
    OutlineNonFinite,
    /// An outline coordinate beyond the scene coordinate limit.
    #[error("outline coordinate exceeds the limit of {limit}")]
    OutlineCoordinateTooLarge {
        /// The limit (`quorra_scene::MAX_COORDINATE`).
        limit: f32,
    },
    /// An image whose dimensions and byte length disagree, or with a zero dimension.
    #[error("image is {width}x{height} but carries {bytes} bytes")]
    ImageInconsistent {
        /// Claimed width.
        width: u32,
        /// Claimed height.
        height: u32,
        /// Actual byte length.
        bytes: usize,
    },
    /// A ramp with no stops.
    #[error("ramp has no stops")]
    RampEmpty,
    /// A ramp stop offset that is NaN, infinite, or outside `0..=1`.
    #[error("ramp stop offset {offset} is outside 0..=1")]
    RampOffsetOutOfRange {
        /// The offending offset.
        offset: f32,
    },
    /// Ramp stops out of ascending order.
    #[error("ramp stops are not in ascending offset order")]
    RampUnordered,
    /// A ramp stop colour outside its range.
    #[error("ramp stop colour is non-finite or outside 0..=1")]
    RampColorInvalid,
}

/// Why a device will not execute a §7.10.5 type 4 program (ADR 0053).
///
/// Three variants for the three questions `function::admit` asks, in its order: is the
/// instruction list executable, can a shader be generated from it, and can an independent
/// evaluation of it be expected to agree with ours. Each carries the refusal that answered
/// no, in that refusal's own vocabulary, because translating one into the other would put a
/// second definition of "a program this device declines" into the tree.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum FunctionProblem {
    /// The instruction list is not structurally executable: empty, past
    /// [`quorra_scene::MAX_PROGRAM_LENGTH`], or carrying a jump that does not move
    /// strictly forward into it.
    #[error("{0}")]
    Structure(quorra_scene::SceneError),
    /// The walk that admits a program refused it — see [`FunctionRefusal`] for the ground.
    ///
    /// [`FunctionRefusal`]: crate::function::FunctionRefusal
    #[error("{0}")]
    Program(crate::function::FunctionRefusal),
    /// An operator whose two implementations may differ reaches one that turns a small
    /// difference into a large one, so no bound on the disagreement between this device
    /// and an independent evaluation can be stated (ADR 0053 §3).
    ///
    /// Refused at the upload rather than reported on the frame: the caller's answer is to
    /// fall back to the raster it builds today, and that is cheap here and expensive after
    /// a page has been drawn.
    #[error(
        "`{inexact}` at {inexact_at} reaches `{amplifier}` at {amplifier_at}, so no bound \
         on the disagreement with an independent evaluation can be stated"
    )]
    NoAgreementBound {
        /// The inexact operator, as ISO 32000-2 Table 42 spells it.
        inexact: &'static str,
        /// Where it is, as an index into the program.
        inexact_at: usize,
        /// The operator whose result its value reaches.
        amplifier: &'static str,
        /// Where that one is.
        amplifier_at: usize,
    },
}

/// Why a shader module or a render pipeline could not be built on this adapter.
///
/// Its own type rather than a [`RenderError`] variant's fields, and deliberately
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

/// Why a [`Presenter`](crate::present::Presenter) will not put a layer on the surface
/// (ADR 0056).
///
/// There is deliberately no variant for an empty texture: `wgpu` will not create one at
/// all (WebGPU requires every extent to be at least 1), so a variant for it would be a
/// refusal no input can reach, and "how often does this happen?" must stay a question
/// with an answer.
///
/// The first four are the contract a layer's texture must satisfy, and they are asked
/// **before** anything is acquired, in the order a frame asks the same questions of
/// [`Target::Texture`](crate::target::Target::Texture) — a refusal must cost no
/// swapchain acquire, because a texture acquired and dropped unpresented wedges the
/// swapchain. The fifth is asked of `wgpu` rather than of the texture, and its variant
/// says why: it is asked of `wgpu`, which is the only thing that can answer it.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum LayerProblem {
    /// A layer texture in another format. The contract is `Rgba8Unorm`, which is what a
    /// [`Target::Texture`](crate::target::Target::Texture) frame renders in (§3).
    #[error("the layer texture is {got:?}; the contract is Rgba8Unorm")]
    Format {
        /// The format the texture actually has.
        got: wgpu::TextureFormat,
    },
    /// A layer texture without `TEXTURE_BINDING` usage.
    ///
    /// The trap this variant exists to name: rendering into a texture needs
    /// `RENDER_ATTACHMENT` and **sampling out of one needs `TEXTURE_BINDING` as well**,
    /// so a texture that has served as a render target every frame can still be
    /// unpresentable. A host asks for both when it creates the texture, once.
    #[error(
        "the layer texture lacks TEXTURE_BINDING usage, so it cannot be sampled; a texture \
         that is both rendered into and presented needs RENDER_ATTACHMENT | TEXTURE_BINDING"
    )]
    NotSampleable,
    /// A layer texture that is not a single-sampled 2D texture with one array layer.
    #[error("the layer texture must be a single-sampled 2D texture with one array layer")]
    Shape,
    /// A placement that is not a finite, invertible transform. The presenter maps each
    /// target pixel back through the inverse, so a degenerate placement has no
    /// arithmetic to do rather than a degenerate picture to draw (§4.7).
    #[error("the layer's placement is not a finite, invertible transform")]
    Placement,
    /// `wgpu` refused to bind the texture — the way a texture belonging to *another*
    /// device arrives, and anything else its validation objects to.
    ///
    /// Asked of `wgpu` because it is the only thing that knows: `wgpu::Texture` in
    /// version 30 exposes its size, format, usage and shape and **not the device that
    /// made it**, so provenance cannot be checked from outside. The bind group is built
    /// inside a validation error scope before the surface is acquired, and what the
    /// scope catches becomes this — a refusal by name rather than the panic an
    /// uncaptured `wgpu` error would otherwise be on this thread.
    ///
    /// **The one case this cannot name**, stated rather than left to be discovered: a
    /// texture from a device of a *different* `wgpu::Instance`. Resource identifiers are
    /// per-instance, so such a texture is not a foreign resource to wgpu-core but a
    /// non-existent one, and the lookup panics before any error scope is consulted. That
    /// is `wgpu` 30's behaviour and it is not new here —
    /// [`Target::Texture`](crate::target::Target::Texture) has the same hole — and the
    /// answer is the one the hoisting constructors already offer: a host with more than
    /// one device builds them all from one instance
    /// ([`Device::headless_with_instance`](crate::device::Device::headless_with_instance)).
    #[error(
        "wgpu refused to bind the layer texture (a texture from another device is this): {detail}"
    )]
    Unbindable {
        /// What `wgpu` said.
        detail: String,
    },
}

/// Why the surface could not provide a texture for this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceProblem {
    /// Acquiring the next frame timed out; the next render reconfigures the surface,
    /// so trying again is the fix. (Reconfigured rather than merely retried because a
    /// timeout can mean a swapchain wedged behind unsignalled acquire semaphores — a
    /// state a retry alone never leaves.)
    Timeout,
    /// The surface no longer matches the window; the next render reconfigures it, so
    /// trying again is the fix.
    Outdated,
    /// The surface is gone and must be recreated with
    /// [`Device::for_surface`](crate::device::Device::for_surface).
    Lost,
    /// The window is occluded; there is nothing to present to right now.
    Occluded,
    /// The surface reported a validation problem.
    Validation,
}

/// Why a frame was refused. A refused frame draws nothing and reports nothing as
/// drawn; each variant names what ran out or what did not hold (§5 of the brief).
#[derive(Debug, Error)]
pub enum RenderError {
    /// The viewport exceeds what this adapter can render.
    #[error("target {width}x{height} exceeds this adapter's limit of {limit} pixels per side")]
    TargetTooLarge {
        /// Requested width.
        width: u32,
        /// Requested height.
        height: u32,
        /// The per-side limit, from [`Device::limits`](crate::device::Device::limits).
        limit: u32,
    },
    /// A zero-size viewport reached a target kind that cannot exist at zero size. (A
    /// zero-size `Readback` is legitimate and yields an empty raster.)
    #[error(
        "a zero-size viewport is renderable only to a Readback target; a {target} cannot exist at zero size"
    )]
    ZeroSizeTarget {
        /// Which target kind refused.
        target: &'static str,
    },
    /// The viewport transform contained NaN or infinity. Refused loudly per §4.7;
    /// never turned into NaN geometry.
    #[error("the viewport transform has a non-finite coefficient")]
    NonFiniteViewportTransform,
    /// A damage rectangle was not a finite, ordered rectangle. Refused rather than
    /// repaired: a malformed damage list means the caller's change tracking broke,
    /// and a guessed region would risk exactly the stale frame damage exists to
    /// prevent (§4.7).
    #[error("damage rect {index} is not a finite, ordered rectangle")]
    InvalidDamage {
        /// Index of the offending rectangle in `Viewport::damage`.
        index: usize,
    },
    /// The frame's rasterised coverage tiles outgrew the scratch image, which the
    /// device dimension bounds on each side. Distinct from the byte budget on
    /// purpose: the two run out independently, and a refusal that names the wrong
    /// one costs the reader the diagnosis.
    ///
    /// **It names the frame as well as the wall** (ADR 0057). `limit` alone is a
    /// property of the adapter, so a caller reading it could not tell a page that asks
    /// for a gigabyte of coverage from an adapter whose textures are small — and a
    /// refused frame has no [`Frame`](crate::frame::Frame), so no
    /// [`Counters`](crate::frame::Counters) either. `sheet` is what the frame had placed
    /// when the tile below would not fit, in the same fields
    /// [`Counters::coverage`](crate::frame::Counters::coverage) reports on a drawn one.
    ///
    /// This is the one budget principle 6's "discoverable before the frame" does not
    /// reach, and saying so is better than implying otherwise: the sheet's height is a
    /// function of the *viewport*, which a `Scene` does not have, so no
    /// [`Scene::cost`](quorra_scene::Scene::cost) can answer it and
    /// [`Limits`](crate::device::Limits) can only state the wall.
    #[error(
        "the frame's rasterised coverage outgrew the {limit}x{limit} scratch image this \
         adapter allows: a {tile_width}x{tile_height} tile would not fit a sheet at {sheet}"
    )]
    ScratchExhausted {
        /// The device's per-side texture limit, which bounds the scratch sheet.
        limit: u32,
        /// What the sheet held when the tile below was refused.
        sheet: CoverageSheet,
        /// Width of the tile that did not fit.
        tile_width: u32,
        /// Height of the tile that did not fit. Compare `sheet.height + tile_height`
        /// against `limit` to see the overshoot, and `sheet.width` against `limit` to
        /// see that the other axis is rarely the binding one.
        tile_height: u32,
    },
    /// The frame's scene-derived allocations would exceed the stated budget. Raised
    /// for instance data (at encode) and for the compositor's internal textures
    /// (before the target is bound) alike; the message names the bytes, not their
    /// lane, because the budget they share is one number.
    #[error("frame needs {needed} scene-derived bytes, over the stated budget of {budget}")]
    FrameBudgetExceeded {
        /// Bytes the scene would need.
        needed: u64,
        /// The configured budget
        /// ([`Options::max_frame_bytes`](crate::startup::Options::max_frame_bytes)).
        budget: u64,
    },
    /// `Target::Surface` or
    /// [`Device::invalidate_surface`](crate::device::Device::invalidate_surface) on a
    /// device constructed with [`Device::headless`](crate::device::Device::headless).
    #[error(
        "this device is headless; construct it with Device::for_surface to render to a surface"
    )]
    NoSurface,
    /// The same two calls on a device whose surface is out with a
    /// [`Presenter`](crate::present::Presenter) (ADR 0056).
    ///
    /// Its own variant rather than [`RenderError::NoSurface`] because the two are
    /// different situations with different fixes, and the host can act on the
    /// difference: this one is answered by
    /// [`Device::attach_presenter`](crate::device::Device::attach_presenter) or by
    /// presenting through the presenter, the other by constructing a different device.
    #[error(
        "this device's surface is out with a Presenter; present through it, or return it \
         with Device::attach_presenter before rendering to Target::Surface"
    )]
    PresenterDetached,
    /// [`Presenter::present`](crate::present::Presenter::present) before any size was
    /// stated. A swapchain is configured for a window's size and this presenter has not
    /// been told one — by
    /// [`Presenter::resize`](crate::present::Presenter::resize), and not by a frame the
    /// device drew before the detach, because there was none.
    ///
    /// Refused rather than guessed: a size invented here configures a swapchain for a
    /// window nobody described (§4.7).
    #[error("this presenter has no size; call Presenter::resize before presenting")]
    PresenterUnsized,
    /// A layer handed to [`Presenter::present`](crate::present::Presenter::present) that
    /// does not satisfy the contract. Nothing was acquired and nothing was presented.
    #[error("layer {index} was refused: {reason}")]
    LayerRefused {
        /// Which layer of the slice, in the order they were given.
        index: usize,
        /// What about it did not hold.
        reason: LayerProblem,
    },
    /// The surface could not provide a texture for this frame.
    #[error("the surface is not renderable right now: {reason:?}")]
    SurfaceUnavailable {
        /// What the surface reported.
        reason: SurfaceProblem,
    },
    /// A `Target::Texture` with the wrong format. The contract is `Rgba8Unorm` (§3:
    /// the boundary format is 8-bit RGBA).
    #[error("target texture is {got:?}; the contract is Rgba8Unorm")]
    TextureFormat {
        /// The format the texture actually has.
        got: wgpu::TextureFormat,
    },
    /// A `Target::Texture` sized differently from the viewport.
    #[error(
        "target texture is {got_width}x{got_height}; the viewport says {need_width}x{need_height}"
    )]
    TextureSize {
        /// The texture's width.
        got_width: u32,
        /// The texture's height.
        got_height: u32,
        /// The viewport's width.
        need_width: u32,
        /// The viewport's height.
        need_height: u32,
    },
    /// A `Target::Texture` without `RENDER_ATTACHMENT` usage.
    #[error("target texture lacks RENDER_ATTACHMENT usage")]
    TextureUsage,
    /// A `Target::Texture` that is not a single-sampled 2D texture with one layer.
    #[error("target texture must be a single-sampled 2D texture with one array layer")]
    TextureShape,
    /// Reading results back from the device failed.
    #[error("reading back from the device failed: {detail}")]
    ReadbackFailed {
        /// What the map reported.
        detail: String,
    },
    /// The device was lost while waiting for the frame.
    #[error("the device was lost while waiting for the frame: {detail}")]
    DeviceLost {
        /// What the poll reported.
        detail: String,
    },
    /// A scene referenced an outline this device has not got — never uploaded,
    /// uploaded to a different device, or already released. Resource ids are
    /// device-scoped (§2.2); a dangling one is a caller bug surfaced by name.
    #[error("the scene references outline {outline:?}, which is not resident on this device")]
    UnknownOutline {
        /// The identifier that was referenced.
        outline: quorra_scene::OutlineId,
    },
    /// A scene referenced an image this device has not got — the same contract as
    /// [`RenderError::UnknownOutline`], per resource family.
    #[error("the scene references image {image:?}, which is not resident on this device")]
    UnknownImage {
        /// The identifier that was referenced.
        image: quorra_scene::ImageId,
    },
    /// A scene referenced a colour ramp this device has not got.
    #[error("the scene references ramp {ramp:?}, which is not resident on this device")]
    UnknownRamp {
        /// The identifier that was referenced.
        ramp: quorra_scene::RampId,
    },
    /// A scene referenced a mesh this device has not got.
    #[error("the scene references mesh {mesh:?}, which is not resident on this device")]
    UnknownMesh {
        /// The identifier that was referenced.
        mesh: quorra_scene::MeshId,
    },
    /// A scene referenced a §7.10.5 program this device has not got — the same contract
    /// as [`RenderError::UnknownOutline`], per resource family.
    #[error("the scene references function {program:?}, which is not resident on this device")]
    UnknownFunction {
        /// The identifier that was referenced.
        program: quorra_scene::FunctionId,
    },
    /// A [`Paint::Function`](quorra_scene::Paint::Function) named a `Range` the program
    /// it references cannot fill, or one a clamp cannot be written against (ADR 0053).
    ///
    /// The program was admitted at upload; what fails here is the pairing of that program
    /// with *this* shading's `Range`, which is the one question about a function paint
    /// that cannot be answered until the two meet.
    /// [`Analysis::admits`](crate::function::Analysis::admits) is how a caller asks it
    /// before a frame.
    #[error(
        "the scene paints with §7.10.5 program {program:?} under a Range it cannot fill: {reason}"
    )]
    FunctionRangeRefused {
        /// The program the paint named.
        program: quorra_scene::FunctionId,
        /// Which pairing failed.
        reason: crate::function::FunctionRefusal,
    },
    /// A pipeline this frame needs could not be built. The frame is refused rather
    /// than drawn without the pass that pipeline was for — a page missing its blit is
    /// exactly the plausible-looking wrong page §5 has a name for.
    #[error("a pipeline this frame needs could not be built: {reason}")]
    PipelineUnavailable {
        /// Which module or pipeline, and what the adapter said.
        #[from]
        reason: PipelineProblem,
    },
    /// [`Frame::into_raster`](crate::frame::Frame::into_raster) on a frame rendered
    /// to a `Surface` or `Texture` target: those pixels are already where the caller
    /// asked.
    #[error("this frame was rendered to a Surface or Texture target and carries no raster")]
    NotAReadbackFrame,
}
