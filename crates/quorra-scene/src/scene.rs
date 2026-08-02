//! The scene, and the builder that produces one.
//!
//! **Skeleton — M2 fills this** (`doc/adr/0003`). This is the centre of the library's
//! design, and `doc/RENDER_LIBRARY.md` §2.3 states the property that decides it:
//!
//! > The single most important property in this document: a `Scene` must contain no
//! > reference to a viewport, a resolution, a device transform, or a target size.
//!
//! Zoom, scroll, window resize and tiled output are all *the same scene at a different
//! viewport*. If building a scene were a function of the target, every zoom step would
//! redo it — and encoding measured 1.1–1.6 ms, flat across a sixteenfold range of
//! resolutions, which is 22% of a thumbnail's frame and 1.5 ms per frame the caller's
//! interpreter is not getting.
//!
//! The corollary, which §2.3 asks to have stated in our documentation rather than left
//! implicit: **a `Scene` is `Send + Sync`, cheap to clone, and building one requires no
//! device.** In this crate that is structural rather than aspirational — there is no
//! device type in scope to require. See `doc/adr/0001`.
//!
//! # Planned signatures
//!
//! ```text
//! pub struct SceneBuilder { /* … */ }
//! pub struct Scene { /* … */ }   // Send + Sync, cheap to clone (Arc inside)
//!
//! impl SceneBuilder {
//!     pub fn fill(&mut self, outline: OutlineId, transform: Affine, rule: FillRule,
//!                 paint: Paint, clip: Option<ClipId>, mask: Option<MaskId>,
//!                 blend: BlendMode, compose: Compose);
//!     pub fn stroke(&mut self, outline: OutlineId, transform: Affine, stroke: &Stroke, …);
//!     /// Not a special case of `fill`: §6.4. A rectangle is exact analytic coverage in a
//!     /// fragment shader — no tiling, no binning, no edge list — and it is what rules,
//!     /// backgrounds, underlines, table cells and *most clips* are.
//!     pub fn rect(&mut self, rect: Rect, transform: Affine, paint: Paint, …);
//!     pub fn image(&mut self, image: ImageId, transform: Affine, alpha: f32, …);
//!     pub fn group(&mut self, group: GroupSpec, body: impl FnOnce(&mut SceneBuilder));
//!
//!     pub fn clip(&mut self, outline: OutlineId, transform: Affine, rule: FillRule,
//!                 parent: Option<ClipId>) -> ClipId;
//!     pub fn mask(&mut self, kind: MaskKind, body: impl FnOnce(&mut SceneBuilder)) -> MaskId;
//!
//!     pub fn finish(self) -> Scene;
//! }
//!
//! impl Scene {
//!     /// §5's second preference: if a limit must exist, it is discoverable *before* the
//!     /// frame. A caller compares this against `Device::limits` and falls back to its CPU
//!     /// backend rather than discovering a blank page afterwards.
//!     pub fn cost(&self) -> Cost;
//! }
//! ```
//!
//! # Properties the M2 tests have to pin
//!
//! 1. **No viewport anywhere.** Not in the type, not in a cached bound, not in a builder
//!    field. A scene built at one scale and rendered at another must be byte-identical to
//!    one built for that scale directly — which is a test, not a comment.
//! 2. **`Send + Sync`, statically.** A compile-time assertion, so that the day someone
//!    adds an `Rc` the build fails rather than the caller's worker thread failing.
//! 3. **Groups are isolated and bounded at 16 deep.** The caller decides isolation
//!    upstream and only emits a group where the computation is provably the isolated one
//!    (§4.4), so we may assume it — and §4.4 asks us to *say* so rather than leave it
//!    implicit, which this line does.
//! 4. **Order-independence.** Every command carries its own absolute transform and clip,
//!    so we may reorder and parallelise; §4.6 forbids the result changing when we do, at
//!    every internal thread count.
//! 5. **A blank scene is a legitimate scene.** Vello's `debug_layers` hands wgpu a
//!    zero-length buffer slice whenever a scene produces no lines, and wgpu panics on it,
//!    which under `panic = "abort"` killed the viewer. An empty `Scene` renders an empty
//!    frame and returns `Ok`.
//!
//! # What a `Scene` may not become
//!
//! Not a scene graph, not a retained widget tree, no animation and no timeline (§9). It is
//! built by an interpreter and thrown away when the page changes. §11.5 asks what one
//! costs to hold, against a target of a dozen resident pages out of a 1 023-page document
//! — so the answer to "should this be cheaper?" is a number, and M2 produces it.
