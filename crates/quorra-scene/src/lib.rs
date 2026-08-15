//! What is to be drawn.
//!
//! This crate is the device-independent half of quorra: a description of marks on a
//! page, with no reference to a viewport, a resolution, a device transform or a target
//! size. Zoom, scroll, window resize and tiled output are all *the same scene at a
//! different viewport*, and a [`Viewport`] lives in `quorra-gpu` for that reason.
//!
//! [`Viewport`]: ../quorra_gpu/viewport/index.html
//!
//! # Why this is a separate crate
//!
//! `doc/RENDER_LIBRARY.md` §2.3 states the property it calls the most important one in
//! the whole brief, and its corollary: a scene is `Send + Sync`, and **building one
//! requires no device**. The caller's content-stream interpreter runs on a worker
//! thread and builds scenes there while the GPU is still initialising.
//!
//! This crate has no dependency on `wgpu`, so that property is enforced by the
//! dependency graph rather than by review: a scene cannot reach a device because no
//! device type is in scope. See `doc/adr/0001`.
//!
//! # State
//!
//! Skeleton. Each module below states what it owns and the signatures it will hold; the
//! two modules whose types *are* the contract — [`blend`] and [`ids`] — are real.
//! `doc/adr/0003` explains what that means and what it forbids, and `doc/PLAN.md` says
//! which milestone fills which module.

#![forbid(unsafe_code)]

pub mod blend;
pub mod error;
pub mod function;
pub mod geom;
pub mod ids;
pub mod image;
pub mod mask;
pub mod mesh;
pub mod paint;
pub mod scene;

pub use blend::{BlendMode, Compose, FillRule};
pub use error::{GroupComposeReason, NonIsolatedReason, SceneError, StagedComposeReason};
pub use function::{FnOp, FnRange, FunctionPaint, MAX_PROGRAM_LENGTH};
pub use geom::{Affine, Point, Rect, Segment, Size, axis_aligned_rect};
pub use ids::{ClipId, ImageId, MaskId, MeshId, OutlineId, RampId, ResourceId};
pub use image::ImageSpec;
pub use mask::{MaskKind, Transfer};
pub use mesh::MeshSpec;
pub use paint::{Color, LineCap, LineJoin, Paint, ShadingKind, Stop, Stroke};
pub use scene::{
    ClipDef, Command, Cost, GroupSpec, ImageFilter, MAX_COORDINATE, MAX_GROUP_DEPTH, MaskDef,
    Scene, SceneBuilder,
};
