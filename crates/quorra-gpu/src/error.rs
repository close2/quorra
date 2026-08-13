//! Every way this crate refuses, in two typed enums.
//!
//! §5 of the brief: a failure is an `Err` that names what overflowed or did not hold,
//! so the caller can act on it — its window falls back to the CPU backend, and a
//! person reading a log can attribute the refusal without reproducing it. There is no
//! catch-all variant in either enum, for the same reason `ReportKind` has no `Other`:
//! a variant that names nothing makes "how often does this happen?" unanswerable.
//!
//! The split follows the two phases of a device's life: [`DeviceError`] is
//! construction (§2.1), [`RenderError`] is a frame (§2.4). A report — the frame *was*
//! drawn, something in it is not as asked — is deliberately neither; that is
//! [`crate::report`].

use thiserror::Error;

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
    #[error(
        "the frame's rasterised coverage outgrew the {limit}x{limit} scratch image this adapter allows"
    )]
    ScratchExhausted {
        /// The device's per-side texture limit, which bounds the scratch sheet.
        limit: u32,
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
