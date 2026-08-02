//! Handles: uploaded once, referenced many times.
//!
//! `doc/RENDER_LIBRARY.md` §2.2 gives the reason these exist at all — **the 107 distinct
//! outlines of one dense page are uploaded once and referenced 5 933 times**, and a zoom
//! must re-upload none of them. Separating upload from scene building is what makes that
//! possible, and an identifier is what a scene carries instead of the data.
//!
//! Two families, and the difference matters:
//!
//! - **Device resources** — [`OutlineId`], [`ImageId`], [`RampId`], [`MeshId`]. Allocated
//!   by a device, released by a device, valid for as long as that device. [`ResourceId`]
//!   is the union, so that one `release` can take any of them.
//! - **Scene-scoped references** — [`ClipId`], [`MaskId`]. Allocated by a scene builder
//!   and meaningful only inside the scene that produced them, because a clip is a path
//!   plus a parent and a mask is a rendered group. They are *not* device resources and so
//!   are deliberately absent from [`ResourceId`].
//!
//! The caller's display list already gives identity for free — every occurrence of a
//! letter on a page is the same `Arc<Path>`, so `Arc::as_ptr` is the key its encoder maps
//! to an [`OutlineId`].

/// A path uploaded to a device, in the scene's own coordinate space.
///
/// The `u32` is public, and deliberately: our caller keys its own
/// pointer-identity-to-handle map by it, and an accessor would only be a longer spelling
/// of the same thing. It is an index into the device that issued it and means nothing
/// anywhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutlineId(pub u32);

/// A decoded image uploaded to a device.
///
/// Straight-alpha RGBA8, row-major, no padding — see §3, and note that the filtering
/// decision is made upstream and reaches us resolved, not as the flag it came from
/// (`doc/PLAN.md`, integration note 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImageId(pub u32);

/// A colour ramp uploaded to a device, for the axial, radial and function-based shadings
/// of ISO 32000-2 §8.7.4.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RampId(pub u32);

/// A pre-rasterised triangle mesh uploaded to a device.
///
/// The caller rasterises meshes itself and shares the result between its backends,
/// because neither of its rasterisers has the primitive and a second copy would drift. We
/// inherit that: we consume the mesh, we do not re-triangulate it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeshId(pub u32);

/// A clip region within one scene: a path, a fill rule and an optional parent, so that a
/// chain is an intersection.
///
/// Chains are deep and repetitive on real pages — the caller's worst page holds 3 608 of
/// them, and its page 6 states one clipping rectangle 303 times and gets one identifier,
/// because its display list deduplicates identical regions. That deduplication is what
/// makes caching clip masks viable at all, and §6.4 is why a rectangular clip should
/// become four floats rather than a mask texture.
///
/// An **empty clip admits nothing**, which is a different thing from an absent clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClipId(pub u32);

/// A soft mask within one scene.
///
/// Not an alpha texture: a transparency group, rendered at device resolution and reduced
/// to mask values by ISO 32000-2 §11.5's Alpha or Luminosity rule, optionally through
/// §11.6.5.1's transfer function. See [`crate::mask`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MaskId(pub u32);

/// Any device resource, so that one release path can take all four.
///
/// §2.2's `Device::release(&mut self, id: impl Into<ResourceId>)`. [`ClipId`] and
/// [`MaskId`] are absent on purpose — they belong to a scene, not to a device, and a
/// scene is dropped whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceId {
    /// An uploaded outline.
    Outline(OutlineId),
    /// An uploaded image.
    Image(ImageId),
    /// An uploaded colour ramp.
    Ramp(RampId),
    /// An uploaded mesh.
    Mesh(MeshId),
}

impl From<OutlineId> for ResourceId {
    fn from(id: OutlineId) -> Self {
        Self::Outline(id)
    }
}

impl From<ImageId> for ResourceId {
    fn from(id: ImageId) -> Self {
        Self::Image(id)
    }
}

impl From<RampId> for ResourceId {
    fn from(id: RampId) -> Self {
        Self::Ramp(id)
    }
}

impl From<MeshId> for ResourceId {
    fn from(id: MeshId) -> Self {
        Self::Mesh(id)
    }
}

#[cfg(test)]
mod tests {
    use super::{ImageId, MeshId, OutlineId, RampId, ResourceId};

    /// Four resource families, four variants, no crossed wires. A `From` that put an
    /// outline in the image slot would release the wrong resource and be visible only as
    /// a later frame drawing something it should not.
    #[test]
    fn each_handle_converts_to_its_own_variant() {
        assert_eq!(
            ResourceId::from(OutlineId(7)),
            ResourceId::Outline(OutlineId(7))
        );
        assert_eq!(ResourceId::from(ImageId(7)), ResourceId::Image(ImageId(7)));
        assert_eq!(ResourceId::from(RampId(7)), ResourceId::Ramp(RampId(7)));
        assert_eq!(ResourceId::from(MeshId(7)), ResourceId::Mesh(MeshId(7)));
    }
}
