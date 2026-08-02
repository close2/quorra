//! Three kinds of target, because there are three kinds of host.
//!
//! **Skeleton — M1 fills this** (`doc/adr/0003`).
//!
//! # Why three, and why this is the first milestone's work
//!
//! §6.1 measured an offscreen frame and found that **between 55% and 92% of it is paid
//! before any of the page is drawn** — 3.48 ms of 6.34 at 1×, 26.73 of 29.13 at 4×. Most
//! of that scales with *bytes*: 26.7 ms for a 32 MB target is about 1.2 GB/s, which is what
//! a mapped readback and a demultiply cost.
//!
//! **A window presenting to a swapchain does not pay the readback at all.** That is why the
//! brief asks for three target kinds rather than one, and why the ranking it gives puts the
//! surface and texture paths first: they delete the largest single item in the frame.
//!
//! # Planned signatures
//!
//! ```text
//! pub enum Target<'a> {
//!     /// Tier 1: we want the pixels. Straight-alpha RGBA8 — the caller's `Raster`.
//!     /// This is the oracle's path and the expensive one; §8's rule about making a cost
//!     /// expensive *in the API* applies to whatever hands these bytes back.
//!     Readback,
//!     /// Tier 2: draw into the surface the device was constructed with.
//!     Surface,
//!     /// Tier 3: a texture the host owns and composites itself.
//!     Texture(&'a wgpu::Texture),
//! }
//! ```
//!
//! # The rule that holds for all three
//!
//! **Render onto transparency, always.** §11.4.7 makes the page group *isolated*, and a
//! finished page is composited onto the medium by the caller after we return it — painting
//! the medium first is a different picture. `Readback` therefore hands back **straight-alpha
//! RGBA8**, converted once at the boundary, which is what PNG and the caller's comparison
//! harness expect.
//!
//! # What M1 has to settle here
//!
//! §11's first question, and it is the reason this module is in the first milestone: *how
//! much of that fixed cost is the readback?* It could not be separated with a wall clock,
//! and the answer changes the priority of everything else — if it is nearly all of it, then
//! tier 2 and tier 3 are the whole performance story and the glyph atlas is a second-order
//! effect. It needs a timestamp query, not a stopwatch.
