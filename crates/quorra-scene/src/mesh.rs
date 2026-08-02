//! Meshes: shading types the caller has already rasterised.
//!
//! [`MeshSpec`] is real as of M2 because `Device::upload_mesh` validates it at upload
//! time; drawing a mesh is M7's work (`doc/PLAN.md`).
//!
//! # Why a mesh arrives as pixels (§1.1 of the brief, integration note 5)
//!
//! The caller's `MeshRaster` is a pre-rasterised triangle mesh — ISO 32000-2
//! §8.7.4.5.5–.7's mesh shadings, rasterised once upstream and shared between both of
//! its backends, because neither rasteriser has the primitive and a second copy would
//! drift. We inherit that: a mesh reaches us as a positioned raster, and we do not
//! re-triangulate it.
//!
//! One consequence the caller's design carries and we state rather than hide: a
//! `MeshRaster` is built at **device resolution** for a specific placement, so unlike
//! outlines and images a mesh upload is viewport-dependent — a zoom re-rasterises and
//! re-uploads its meshes. That is the cost of the shared-rasteriser correctness
//! argument, it is the caller's decision (trap 2: taken once, upstream), and meshes
//! are rare enough on real pages that §6 never mentions them in a hot path.

use crate::image::ImageSpec;

/// A pre-rasterised mesh as uploaded to a device: straight-alpha RGBA8 samples with a
/// device-space anchor, mirroring the caller's `MeshRaster`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshSpec {
    /// Device x of the raster's first column.
    pub left: i32,
    /// Device y of the raster's first row.
    pub top: i32,
    /// The samples. Must be consistent per [`ImageSpec::is_consistent`].
    pub image: ImageSpec,
}

impl MeshSpec {
    /// The bytes this mesh costs while resident, for the resource budget.
    #[must_use]
    pub fn byte_size(&self) -> u64 {
        self.image.byte_size()
    }
}
