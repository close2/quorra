//! Building soft masks on the device.
//!
//! **Skeleton — M6 fills this** (`doc/adr/0003`). The scene-side vocabulary is
//! [`quorra_scene::mask`]; this module is the pass that evaluates it.
//!
//! [`quorra_scene::mask`]: ../quorra_scene/mask/index.html
//!
//! # What has to happen here, and what must stop happening
//!
//! Today's Vello-based backend renders each mask group to its own texture, **reads it back
//! to the CPU**, converts it with the caller's `SoftMask::value`, and uploads it again as an
//! alpha layer — per mask, per frame. §4.2 wants the reduction on the device:
//! `MaskKind::Alpha`, `MaskKind::Luminosity { backdrop }`, and an optional 256-entry
//! transfer table, with the mask group built through the same scene builder as everything
//! else.
//!
//! # The part that makes this a correctness deliverable
//!
//! `SoftMask::value` is shared by both of the caller's backends *on purpose*, so that what
//! the pixels mean is decided once. Moving the reduction onto the device makes our shader a
//! **second implementation of that function**, and §4.2 requires it to agree **to the
//! byte**. So the conformance test is part of M6's definition of done:
//!
//! - every one of the 256 possible mask bytes, through both rules;
//! - a luminosity mask with a non-black backdrop, because the default hides the term that
//!   makes the area outside a mask group's marks mask everything away;
//! - a transfer table that is not the identity, sampled at both endpoints.
//!
//! # And the part that is easy to get subtly wrong
//!
//! §11.5.3 composites the group onto **a fully opaque backdrop of a specified colour** and
//! then takes the luminosity of the *result*. Taking the luminosity of the group and then
//! compositing that against the backdrop's luminosity is a different number wherever the
//! group is partly transparent, and it produces a plausible picture. The clause's order is
//! the order, and the shader comment says so with the clause number beside it.
