//! Pipelines and shaders: how few, and how late.
//!
//! **Skeleton — M1 fills this, and every milestone after adds to it** (`doc/adr/0003`).
//!
//! # The contract
//!
//! §7 makes this module a startup-latency problem before it is a rendering problem. The
//! caller renders page one on its CPU backend *while we initialise*, so what we cost before
//! the first frame is what decides whether the handover is invisible.
//!
//! - **Few pipelines.** Vello compiles about twenty compute shaders up front. §7's estimate
//!   for a hybrid design is five or six — glyph quads, rectangle fills, general path
//!   coverage, composite, mask build, image blit — and **only the first three are needed for
//!   a page of text**.
//! - **Compiled lazily.** Shadings, meshes and the exotic blend modes compile on first use.
//!   `Device::headless` may return before every pipeline exists, with the rest compiled in
//!   the background and `is_warm` available to whoever wants to ask.
//! - **A pipeline cache we can persist.** If the driver hands us a binary blob, the caller
//!   saves it and hands it back next launch — and **is told when it was rejected**, because
//!   a silently recompiled cache is a startup regression nobody can attribute.
//! - **Timed, and split.** Adapter enumeration, device creation and pipeline compilation are
//!   three numbers, not one, because they have three different causes and three different
//!   fixes.
//!
//! # Shaders are code, and this project's rules apply to them
//!
//! WGSL lives in this crate, in its own files, and every function implementing a normative
//! requirement carries its clause number in a comment — a WGSL blend function is not exempt
//! from CLAUDE.md principle 5 just because it is not Rust. Principle 4 bites hardest here: a
//! shader is write-only code unless the invariant it relies on is stated beside it.
//!
//! # A dependency failure to design out rather than inherit
//!
//! Turning on a feature flag for one effect brought another: Vello's `debug_layers` also
//! makes it hand wgpu a **zero-length buffer slice** whenever a scene produces no lines, and
//! wgpu panics on it, which under `panic = "abort"` killed the viewer. **A blank scene is a
//! legitimate scene**, an empty buffer is a legitimate buffer, and no diagnostic feature of
//! ours may change what a frame does — only what it reports about itself.
