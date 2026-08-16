//! Why one frame was refused — and why one present was, which is a frame's last step
//! happening somewhere else (ADR 0056).
//!
//! One enum, and an enum is one item that cannot be split further, so this is the
//! largest file the seam admits: `Device::render`, `Device::render_retained`,
//! `Presenter::present` and `Frame::into_raster` return it and nothing else does. Its
//! variants are in no particular order to a reader, but the
//! *order they are raised in* is a contract stated where they are raised — `device/bound.rs`
//! for a target's four questions, `present/layer.rs` for a layer's, and both before
//! anything is acquired.
//!
//! Four variants delegate the *why* to a vocabulary of its own —
//! [`LayerProblem`], [`SurfaceProblem`], [`PipelineProblem`] and
//! [`FunctionRefusal`](crate::function::FunctionRefusal) — so that what a caller matches
//! on stays a list of situations while the reasons stay countable.

use thiserror::Error;

use crate::frame::CoverageSheet;

use super::layer::LayerProblem;
use super::pipeline::PipelineProblem;
use super::surface::SurfaceProblem;

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
