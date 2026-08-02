//! Soft masks: a rendered group reduced by a stated rule.
//!
//! # The contract
//!
//! A soft mask is not an alpha texture. ISO 32000-2 §11.5 makes it a **transparency
//! group, rendered at device resolution**, reduced to mask values by one of two
//! rules:
//!
//! - **Alpha** (§11.5.2): the group's alpha, its colours ignored.
//! - **Luminosity** (§11.5.3): the group composited onto *a fully opaque backdrop of
//!   a specified colour*, then the luminosity of the result. The backdrop colour is
//!   the mask's own, resolved upstream (the caller's default is black — which is what
//!   makes the area outside a mask group's marks mask everything away).
//!
//! Then optionally §11.6.5.1's `/TR`, which reaches us as a **256-entry lookup
//! table** rather than a function: a mask value is one byte, so the table holds every
//! value the function can be asked about, and sampling it is exact.
//!
//! # Why the reduction is a conformance item
//!
//! The caller's `SoftMask::value` is shared by both of its backends *on purpose*, so
//! that what the pixels mean is decided once. Our device-side reduction
//! (`quorra-gpu`'s `reduce.wgsl`) is a second implementation of that function and
//! must agree with it **to the byte** — the conformance test ships with the shader
//! (M6), not after it. The clause's order is the order: composite onto the opaque
//! backdrop *first*, then take the luminosity of the result; the other order is a
//! different number wherever the group is partly transparent, and it produces a
//! plausible picture.

use crate::paint::Color;

/// Which of §11.5's two rules turns the rendered mask group into mask values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MaskKind {
    /// §11.5.2: the group's alpha; colours ignored.
    Alpha,
    /// §11.5.3: source-over onto this fully opaque backdrop, then the luminosity of
    /// the result. The backdrop arrives resolved (the default-black rule is applied
    /// upstream, once — a default here would be a second place the rule lives).
    Luminosity {
        /// §11.6.5.1's `/BC`, resolved to device RGB; its alpha is ignored (the
        /// clause makes the backdrop fully opaque).
        backdrop: Color,
    },
}

/// §11.6.5.1's `/TR`, sampled exactly: index by the derived mask byte, take the
/// value. The caller samples its `Transfer` onto these 256 entries (integration
/// note 3 in `doc/PLAN.md`); both endpoints are included by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer(pub [u8; 256]);

impl Transfer {
    /// The identity table: every byte maps to itself, the meaning of an absent `/TR`.
    #[must_use]
    pub fn identity() -> Self {
        let mut table = [0_u8; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            // The index of a 256-element array is exactly a byte.
            #[allow(clippy::cast_possible_truncation)]
            {
                *slot = i as u8;
            }
        }
        Self(table)
    }

    /// The mask value for a derived alpha or luminosity byte.
    #[must_use]
    pub fn apply(&self, value: u8) -> u8 {
        self.0[usize::from(value)]
    }
}

#[cfg(test)]
mod tests {
    use super::Transfer;

    /// The identity table is the identity at both endpoints and in between —
    /// §11.6.5.1's sampling is inclusive of both ends (integration note 3).
    #[test]
    fn identity_is_identity_at_both_endpoints() {
        let transfer = Transfer::identity();
        assert_eq!(transfer.apply(0), 0);
        assert_eq!(transfer.apply(128), 128);
        assert_eq!(transfer.apply(255), 255);
    }
}
