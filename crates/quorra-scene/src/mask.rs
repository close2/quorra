//! Soft masks: a rendered group reduced by a stated rule.
//!
//! **Skeleton — M6 fills this** (`doc/adr/0003`).
//!
//! # The contract
//!
//! A soft mask is not an alpha texture. ISO 32000-2 §11.5 makes it a **transparency
//! group, rendered at device resolution**, reduced to mask values by one of two rules:
//!
//! - **Alpha** (§11.5.2): the group's alpha, its colours ignored.
//! - **Luminosity** (§11.5.3): the group composited onto *a fully opaque backdrop of a
//!   specified colour*, then the luminosity of the result. The backdrop colour is the
//!   mask's own, defaulting to black — which is what makes the area outside a mask group's
//!   marks mask everything away.
//!
//! Then optionally §11.6.5.1's `/TR`, which reaches us as a **256-entry lookup table**
//! rather than a function: a mask value is one byte, so the table holds every value the
//! function can be asked about, and sampling it is exact.
//!
//! # Why this is a specification item and not an optimisation
//!
//! Today's Vello-based backend renders each mask group to its own texture, **reads it
//! back to the CPU**, converts it with the caller's `SoftMask::value`, and uploads it
//! again as an alpha layer — per mask, per frame. §4.2 asks for the reduction to happen on
//! the device instead.
//!
//! The catch is the reason this is M6 work rather than a later performance pass:
//! `SoftMask::value` is shared by both of the caller's backends *on purpose*, so that what
//! the pixels mean is decided once. Moving the reduction onto the device makes our shader a
//! second implementation of that function, and it must agree with theirs **to the byte**.
//! The conformance test for exactly that ships with the shader, not after it.
//!
//! # Planned signatures
//!
//! ```text
//! pub enum MaskKind {
//!     /// §11.5.2.
//!     Alpha,
//!     /// §11.5.3. The backdrop defaults to black at the caller, not here — a default
//!     /// here would be a second place the rule lives.
//!     Luminosity { backdrop: Color },
//! }
//!
//! /// §11.6.5.1's `/TR`, sampled exactly: index by the mask byte, take the value.
//! pub struct Transfer(pub [u8; 256]);
//!
//! // Built through the same SceneBuilder as everything else (§4.2), which is what makes
//! // a mask group able to contain a group, an image or a mask of its own:
//! //   let id = builder.mask(MaskKind::Luminosity { backdrop }, |b| { … });
//! ```
//!
//! # Open until M6
//!
//! Whether the transfer table belongs on the mask or on the *use* of the mask. §11.6.5.1
//! attaches `/TR` to the soft-mask dictionary, which argues for the mask; a device that
//! caches a reduced mask would rather key the cache without it. The clause wins unless a
//! measurement is large enough to justify an ADR arguing otherwise.
