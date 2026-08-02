//! Meshes: shading types the caller has already rasterised.
//!
//! **Skeleton — M7 fills this** (`doc/adr/0003`).
//!
//! # The contract
//!
//! ISO 32000-2 §8.7.4.5.5 to §8.7.4.5.7 define the free-form and lattice-form Gouraud
//! triangle meshes and the two Coons/tensor patch types. Neither of the caller's
//! rasterisers has the primitive, so it evaluates the patches itself and hands both
//! backends **the same pre-rasterised triangle mesh** — a decision it took (its ADR 0051)
//! precisely so that two implementations could not drift.
//!
//! We inherit that decision rather than re-taking it: we consume the mesh, we do not
//! subdivide, re-triangulate or re-interpolate it. What we own is drawing a lot of
//! flat-shaded triangles quickly, which is the one part of this library that a GPU is
//! trivially good at.
//!
//! # Planned signatures
//!
//! ```text
//! pub struct MeshSpec<'a> {
//!     pub triangles: &'a [Triangle],
//! }
//!
//! /// Vertices in the scene's own space, colours already device RGB.
//! pub struct Triangle {
//!     pub vertices: [Point; 3],
//!     pub colors: [Color; 3],
//! }
//! ```
//!
//! # Open until M7
//!
//! Whether a mesh is a resource ([`MeshId`], uploaded once) or geometry inline in a
//! command. §2.2 says resource, and a mesh is large, so that is the presumption; the case
//! that could argue otherwise is a page whose every mesh is used once, where the upload is
//! pure cost. Measure it on the corpus before deciding, and if the answer is "both",
//! prefer the resource and say why in an ADR — two paths for one primitive is how a
//! backend acquires a rarely-taken branch nobody tests.
//!
//! [`MeshId`]: crate::ids::MeshId
