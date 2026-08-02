//! A GPU renderer for **documents**.
//!
//! Not a general 2D vector renderer pointed at documents — the other way round. Its fast
//! paths assume that most of a page is the same few glyph outlines repeated at many
//! sub-pixel phases plus axis-aligned rectangles, and it treats general curve filling as
//! the rare case rather than the uniform one. It expresses the transparency model of
//! ISO 32000-2 clause 11 natively, because that is the part an SVG-shaped renderer cannot
//! be patched into.
//!
//! # State
//!
//! **Skeleton.** The requirements are in `doc/RENDER_LIBRARY.md`, the order of work in
//! `doc/PLAN.md`, and `doc/adr/0003` says what a module may contain before the milestone
//! that fills it. Nothing renders yet, and nothing pretends to.
//!
//! # The shape of the API, once there is one
//!
//! Two halves, and the split is enforced by the dependency graph rather than by review
//! (`doc/adr/0001`):
//!
//! - [`scene`] — what is to be drawn. **No viewport, no resolution, no device transform, no
//!   target size**, and no device: a scene is `Send + Sync`, cheap to clone, and built on
//!   whatever thread interpreted the page. Zoom, scroll, resize and tiled output are all the
//!   same scene at a different viewport.
//! - [`gpu`] — the device, the pipelines, the atlas, and a frame that tells the truth about
//!   itself.
//!
//! ```text
//! let mut device = Device::headless(&Options::default())?;   // background thread welcome
//! let outline = device.upload_outline(&segments)?;           // once; referenced thousands of times
//!
//! let mut builder = SceneBuilder::new();                     // no device needed
//! builder.fill(outline, transform, FillRule::NonZero, Paint::Solid(black),
//!              None, None, BlendMode::Normal, Compose::SrcOver);
//! let scene = builder.finish();
//!
//! let frame = device.render(&scene, &viewport, Target::Surface)?;
//! for report in frame.reports() { /* what could not be drawn as asked */ }
//! ```
//!
//! # Two promises worth reading before the rest
//!
//! **A frame is drawn, or it is refused.** Never a blank target under an `Ok`. Limits are
//! discoverable before the frame, a failure is an `Err` that names what overflowed, and
//! anything drawn differently from what the scene asked is a `Report` — never a silent
//! approximation.
//!
//! **We do not colour-manage, load fonts, shape text, lay out anything, or parse any
//! document format.** Colours arrive as device RGB and glyphs arrive as positioned outlines,
//! because the caller has already decided those questions and deciding them twice is how two
//! renderers disagree. The full list is §9 of the brief.

#![forbid(unsafe_code)]

/// What is to be drawn: the device-independent scene model.
pub use quorra_scene as scene;

/// The device: pixels, and the measurements that make a frame accountable.
pub use quorra_gpu as gpu;

pub use quorra_scene::{
    BlendMode, ClipId, Compose, FillRule, ImageId, MaskId, MeshId, OutlineId, RampId, ResourceId,
};
