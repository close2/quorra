//! Images: decoded pixels, and a filtering decision that is not ours.
//!
//! [`ImageSpec`] is real as of M2 because `Device::upload_image` validates it at
//! upload time; the *drawing* of images — the image lane's quad, the alpha, the
//! per-command filter — is M7's work (`doc/PLAN.md`).
//!
//! # The filtering decision (§4.5 of the brief, integration note 1 in `doc/PLAN.md`)
//!
//! ISO 32000-2 §8.9.5.3's `/Interpolate` and the caller's documented
//! area-averaging departure from §10.7.4 are decisions settled upstream, and in the
//! caller's tree they are *methods of the placement*, not flags of the image —
//! `is_smoothed(placement)` depends on how large the image is drawn. What that means
//! for this API: the uploaded resource carries **pixels only**, and the resolved
//! filter for a given placement arrives on the image *command* (M7), which is the
//! thing that knows its placement. An uploaded image is placement-independent, which
//! is what lets one upload serve every zoom level.

use std::sync::Arc;

/// A decoded image as uploaded to a device: straight-alpha RGBA8, row-major, top row
/// first, no padding (§3 of the brief) — the caller's own `Image` layout.
///
/// The samples sit behind an `Arc` because the caller already holds them behind one;
/// an upload borrows the same allocation rather than copying it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSpec {
    /// Width in samples. Nonzero for a consistent spec.
    pub width: u32,
    /// Height in samples. Nonzero for a consistent spec.
    pub height: u32,
    /// `width × height × 4` bytes of straight-alpha RGBA8.
    pub data: Arc<[u8]>,
}

impl ImageSpec {
    /// Whether the dimensions and the buffer length agree, and neither dimension is
    /// zero.
    ///
    /// Checked at upload: a mismatch means an indexing bug upstream, and a device that
    /// trusted the dimensions would read past a short buffer or render garbage — the
    /// caller's own `Image::is_consistent` guards the same boundary for the same
    /// reason.
    #[must_use]
    pub fn is_consistent(&self) -> bool {
        let expected = (self.width as usize)
            .saturating_mul(self.height as usize)
            .saturating_mul(4);
        self.width > 0 && self.height > 0 && self.data.len() == expected
    }

    /// The bytes this image costs while resident, for the resource budget.
    #[must_use]
    pub fn byte_size(&self) -> u64 {
        self.data.len() as u64
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::ImageSpec;

    /// Consistency is dimensions-times-four bytes exactly, and no zero dimension —
    /// §4.7's boundary check, defined once.
    #[test]
    fn consistency_checks_dimensions_and_length() {
        let good = ImageSpec {
            width: 2,
            height: 3,
            data: Arc::from(vec![0_u8; 24].as_slice()),
        };
        assert!(good.is_consistent());
        let short = ImageSpec {
            data: Arc::from(vec![0_u8; 23].as_slice()),
            ..good.clone()
        };
        assert!(!short.is_consistent());
        let zero = ImageSpec {
            width: 0,
            ..good.clone()
        };
        assert!(!zero.is_consistent());
    }
}
