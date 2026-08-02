//! The device: adapter, queue, pipelines, and the resources a scene refers to.
//!
//! **Skeleton — M1 fills this** (`doc/adr/0003`).
//!
//! # The contract
//!
//! §2.1, and the sentence that constrains the whole design:
//!
//! > **`Device` must be constructible on a background thread and must not need one.** Page
//! > one of a document renders on the CPU backend while the GPU initialises, which is
//! > `CLAUDE.md`'s startup rule; a `Device::headless` that blocks a main thread for 200 ms
//! > is a design we cannot use even if every frame afterwards is free.
//!
//! **Headless is a first-class citizen, not an afterthought** — it is the form the caller's
//! test suite and its correctness oracle use, so it is the form developed first.
//!
//! # Planned signatures
//!
//! ```text
//! pub struct Device { /* … */ }
//!
//! impl Device {
//!     /// No window, no surface.
//!     pub fn headless(options: &Options) -> Result<Self, DeviceError>;
//!     /// `raw-window-handle` and nothing more specific (integration note 4 in PLAN.md).
//!     pub fn for_surface(handle: impl HasWindowHandle, options: &Options)
//!         -> Result<Self, DeviceError>;
//!
//!     pub fn description(&self) -> &str;   // adapter name, for reports and goldens
//!     pub fn limits(&self) -> Limits;      // what this adapter can actually do
//!
//!     /// §7: the device may return before every pipeline exists, so a caller that cares
//!     /// can ask. A caller that does not ask still gets correct frames, more slowly.
//!     pub fn is_warm(&self) -> bool;
//!
//!     pub fn upload_outline(&mut self, path: &[Segment]) -> Result<OutlineId, DeviceError>;
//!     pub fn upload_image(&mut self, image: &ImageSpec) -> Result<ImageId, DeviceError>;
//!     pub fn upload_ramp(&mut self, stops: &[Stop]) -> Result<RampId, DeviceError>;
//!     pub fn upload_mesh(&mut self, mesh: &MeshSpec) -> Result<MeshId, DeviceError>;
//!     pub fn release(&mut self, id: impl Into<ResourceId>);
//!
//!     pub fn render(&mut self, scene: &Scene, viewport: &Viewport, into: Target)
//!         -> Result<Frame, RenderError>;
//! }
//!
//! pub struct Options {
//!     /// §4.5's fifth decision, the one that is ours to expose: the sub-pixel quantum of
//!     /// the glyph cache. Settable, documented, and switchable off. 1/16 of a pixel
//!     /// reused 5.0x on a dense page and left the oracle's verdicts unmoved; 1/8
//!     /// contradicted pages. Default 1/16, and a silent quantum would be a change to
//!     /// where the text sits that nobody could attribute.
//!     pub glyph_quantum: Option<GlyphQuantum>,
//!     /// §6.3: the atlas is sized from a budget we are given, not from a constant.
//!     pub atlas_budget: usize,
//!     /// §7: a driver blob from a previous launch, and a report when it is rejected.
//!     pub pipeline_cache: Option<Vec<u8>>,
//! }
//!
//! pub struct Limits { /* … */ }
//!
//! pub enum DeviceError { /* names what was unavailable, never a bare "failed" */ }
//! pub enum RenderError { /* §5.3: names what overflowed, so a caller can fall back */ }
//! ```
//!
//! # Startup rules M1 is held to
//!
//! - **Few pipelines, compiled lazily.** Vello compiles about twenty compute shaders up
//!   front. A hybrid design plausibly needs five or six — glyph quads, rectangle fills,
//!   general path coverage, composite, mask build, image blit — and only the first three
//!   are needed for a page of text. Shadings, meshes and the exotic blend modes compile on
//!   first use.
//! - **Report what startup cost**, split into adapter enumeration, device creation and
//!   pipeline compilation, because that number goes into a CI gate in the same milestone
//!   that first produces it.
//!
//! # Errors, and the shape they must have
//!
//! §5 ranks the answers, and the ranking is a requirement rather than a preference:
//!
//! 1. **Memory that grows** — count then allocate, or a growable arena with a fence that
//!    reruns on overflow. Then a page is drawn or the *allocation* fails.
//! 2. **A limit that must exist is discoverable before the frame**, through
//!    `Device::limits` and `Scene::cost()`.
//! 3. **A failure is an `Err` that names what overflowed**, so the caller falls back to its
//!    CPU backend — which its window already knows how to do.
