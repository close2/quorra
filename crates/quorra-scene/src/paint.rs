//! Colours, paints and stroke parameters.
//!
//! **Skeleton — M2 fills this, M7 the shading half** (`doc/adr/0003`).
//!
//! # The contract
//!
//! - **Colours reaching us are already device RGB.** Colour management happens upstream:
//!   ICC profiles, `DeviceCMYK`, `CalRGB`, `Separation`, rendering intents, black point
//!   compensation. `ColourSpace::to_rgb` in the caller's tree is the only place a colour
//!   becomes RGB and adding a second one is forbidden. §3 is blunt about the consequence:
//!   *if we offer colour management it will not be used, and if it is on by default the
//!   library cannot be used at all.*
//! - **Straight alpha at the boundary, premultiplied internally.** Converting once at the
//!   boundary is cheaper than converting per comparison, and it is what PNG and the
//!   caller's harness expect.
//! - **Stroke widths are given, not derived.** §8.4.3.2 with §10.7.5 makes a `0 w` line
//!   one device pixel, and the caller resolves that into `Stroke::device_width` before we
//!   see it. `tiny-skia` happened to do the right thing, so the rule went unwritten and
//!   **every zero-width line was invisible on the GPU for fifteen sessions**. We take the
//!   width we are given.
//! - **Dashing is already done.** The caller dashes its own paths, including zero-length
//!   dashes whose caps face along the path — Skia's dasher loses that direction and paints
//!   them upright, and on a diagonal dotted line the two answers cover different pixels.
//!   What we must not do is undo it, so a stroke that reaches us has no dash array at all.
//! - **Degenerate subpaths are already split.** §8.5.3.2 makes a zero-length subpath a dot
//!   under round caps and *nothing* under butt or square; three libraries gave three
//!   answers and none was the standard's. We draw what we are given.
//!
//! # Planned signatures
//!
//! ```text
//! /// Straight alpha, device RGB, 0..=1 per component.
//! pub struct Color { pub r: f32, pub g: f32, pub b: f32, pub a: f32 }
//!
//! pub enum Paint {
//!     Solid(Color),
//!     /// §8.7.4.5.2 axial, .3 radial, .4 function-based: a ramp plus a mapping.
//!     Shading { ramp: RampId, kind: ShadingKind },
//!     /// §8.7.4.5.5-.7, pre-rasterised by the caller and shared between its backends.
//!     Mesh(MeshId),
//! }
//!
//! pub struct Stroke {
//!     /// Already resolved to device space by the caller (§4.5).
//!     pub width: f32,
//!     pub cap: LineCap,
//!     pub join: LineJoin,
//!     pub miter_limit: f32,
//!     // No dash array, and that absence is load-bearing: see above.
//! }
//!
//! pub enum LineCap  { Butt, Round, Square }        // §8.4.3.3
//! pub enum LineJoin { Miter, Round, Bevel }        // §8.4.3.4
//! ```
//!
//! # Open until M7
//!
//! Whether `ShadingKind` carries the shading's own geometry (the axis, the two circles,
//! the domain and the `/Extend` pair) or whether a device resolves it at upload time
//! beside the ramp. The second is fewer bytes per command and one more resource; the
//! first keeps a scene independent of a device for one more thing. It is a measurement,
//! and it belongs in an ADR.
