//! The device: pixels, and the truth about how they were made.
//!
//! This crate turns a [`Scene`] plus a viewport into a frame, on a GPU, through `wgpu`.
//! It is the half of quorra that knows about resolution — and the scene crate beside it
//! is the half that must never learn (`doc/adr/0001`).
//!
//! [`Scene`]: ../quorra_scene/scene/index.html
//!
//! # The two rules that outrank everything else here
//!
//! **A frame is drawn, or it is refused. There is no third state.** The library this one
//! replaces sizes its GPU working buffers from a table of constants whose own comment says
//! they were "hand picked to accommodate the vello test scenes"; a scene needing more
//! overflows them *on the device*, which sets a flag, stops filling, and returns `Ok(())`
//! over a blank target. A page of ISO 32000-2 fitted to a laptop window is such a scene —
//! it needed 4% more tile records than the buffer held. A person reported a black page and
//! nothing in the test suite could see it. So: memory that grows, limits that are
//! discoverable before the frame, and failures that are an `Err` naming what overflowed.
//!
//! **Whatever a [`Frame`] says about itself must be true.** A window in the caller's tree
//! once answered "presented" when its GPU path had refused the page, so the core recorded
//! the page as shown, never asked again, and kept the *previous* page under a title bar
//! naming the new one.
//!
//! [`Frame`]: frame
//!
//! # State
//!
//! All nine milestones are implemented; `doc/PLAN.md`'s "Where we are" is the current
//! state and `doc/adr/` the decisions behind it.

#![forbid(unsafe_code)]

pub mod atlas;
mod census;
mod compose;
mod compute;
pub mod device;
mod encode;
pub mod error;
pub mod frame;
pub mod function;
mod instrument;
mod keyhash;
mod layers;
pub mod mask;
mod outline;
mod pane;
pub mod pipeline;
pub mod present;
mod raster;
mod readback;
pub mod report;
mod resources;
pub mod retained;
mod shaders;
pub mod startup;
mod surface;
pub mod target;
mod timing;
pub mod viewport;
mod winding;

pub use device::{Device, Limits};
pub use error::{
    DeviceError, LayerProblem, PipelineProblem, RenderError, ResourceProblem, SurfaceProblem,
};
pub use frame::{
    Counters, CoverageSheet, EncodeSource, Frame, LaneCounts, Raster, TimingProvenance, Timings,
};
pub use present::{ForeignPresenter, Layer, PresentCost, Presenter};
pub use report::{Report, ReportKind};
pub use retained::RetainedScene;
pub use startup::{
    Coverage, DEFAULT_ATLAS_BUDGET, DEFAULT_COVERAGE_SAMPLES, DEFAULT_GLYPH_QUANTUM,
    DEFAULT_MAX_FRAME_BYTES, DEFAULT_MAX_RESOURCE_BYTES, Options, StartupTimings, WarmUp,
    create_instance, create_instance_with,
};
pub use target::Target;
pub use viewport::Viewport;

/// The exact `wgpu` this crate was built against, re-exported so a tier-3 host can
/// name the types [`Device::wgpu`] hands it — and a windowing host the
/// [`wgpu::SurfaceTarget`] that [`Device::for_surface`] takes — without a second
/// `wgpu` dependency whose version could skew (integration note 4 in
/// `doc/PLAN.md`).
pub use wgpu;
