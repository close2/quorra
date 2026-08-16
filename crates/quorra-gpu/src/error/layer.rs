//! What a layer handed to a presenter did not satisfy (ADR 0056).
//!
//! The vocabulary of
//! [`RenderError::LayerRefused`](crate::error::RenderError::LayerRefused), and the only
//! refusal in this crate whose *order* is part of its contract: the four questions about
//! the texture are asked before the swapchain is touched, because a surface texture
//! acquired and then dropped unpresented wedges the swapchain. The type's own comment
//! carries that order and the one case it cannot name.

use thiserror::Error;

/// Why a [`Presenter`](crate::present::Presenter) will not put a layer on the surface
/// (ADR 0056).
///
/// There is deliberately no variant for an empty texture: `wgpu` will not create one at
/// all (WebGPU requires every extent to be at least 1), so a variant for it would be a
/// refusal no input can reach, and "how often does this happen?" must stay a question
/// with an answer.
///
/// The first four are the contract a layer's texture must satisfy, and they are asked
/// **before** anything is acquired, in the order a frame asks the same questions of
/// [`Target::Texture`](crate::target::Target::Texture) — a refusal must cost no
/// swapchain acquire, because a texture acquired and dropped unpresented wedges the
/// swapchain. The fifth is asked of `wgpu` rather than of the texture, and its variant
/// says why: it is asked of `wgpu`, which is the only thing that can answer it.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum LayerProblem {
    /// A layer texture in another format. The contract is `Rgba8Unorm`, which is what a
    /// [`Target::Texture`](crate::target::Target::Texture) frame renders in (§3).
    #[error("the layer texture is {got:?}; the contract is Rgba8Unorm")]
    Format {
        /// The format the texture actually has.
        got: wgpu::TextureFormat,
    },
    /// A layer texture without `TEXTURE_BINDING` usage.
    ///
    /// The trap this variant exists to name: rendering into a texture needs
    /// `RENDER_ATTACHMENT` and **sampling out of one needs `TEXTURE_BINDING` as well**,
    /// so a texture that has served as a render target every frame can still be
    /// unpresentable. A host asks for both when it creates the texture, once.
    #[error(
        "the layer texture lacks TEXTURE_BINDING usage, so it cannot be sampled; a texture \
         that is both rendered into and presented needs RENDER_ATTACHMENT | TEXTURE_BINDING"
    )]
    NotSampleable,
    /// A layer texture that is not a single-sampled 2D texture with one array layer.
    #[error("the layer texture must be a single-sampled 2D texture with one array layer")]
    Shape,
    /// A placement that is not a finite, invertible transform. The presenter maps each
    /// target pixel back through the inverse, so a degenerate placement has no
    /// arithmetic to do rather than a degenerate picture to draw (§4.7).
    #[error("the layer's placement is not a finite, invertible transform")]
    Placement,
    /// `wgpu` refused to bind the texture — the way a texture belonging to *another*
    /// device arrives, and anything else its validation objects to.
    ///
    /// Asked of `wgpu` because it is the only thing that knows: `wgpu::Texture` in
    /// version 30 exposes its size, format, usage and shape and **not the device that
    /// made it**, so provenance cannot be checked from outside. The bind group is built
    /// inside a validation error scope before the surface is acquired, and what the
    /// scope catches becomes this — a refusal by name rather than the panic an
    /// uncaptured `wgpu` error would otherwise be on this thread.
    ///
    /// **The one case this cannot name**, stated rather than left to be discovered: a
    /// texture from a device of a *different* `wgpu::Instance`. Resource identifiers are
    /// per-instance, so such a texture is not a foreign resource to wgpu-core but a
    /// non-existent one, and the lookup panics before any error scope is consulted. That
    /// is `wgpu` 30's behaviour and it is not new here —
    /// [`Target::Texture`](crate::target::Target::Texture) has the same hole — and the
    /// answer is the one the hoisting constructors already offer: a host with more than
    /// one device builds them all from one instance
    /// ([`Device::headless_with_instance`](crate::device::Device::headless_with_instance)).
    #[error(
        "wgpu refused to bind the layer texture (a texture from another device is this): {detail}"
    )]
    Unbindable {
        /// What `wgpu` said.
        detail: String,
    },
}
