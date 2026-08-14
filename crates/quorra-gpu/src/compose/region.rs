//! Where a plan's pixels are: the frame's rectangle arithmetic, and nothing that
//! touches a device.
//!
//! ADR 0036 made a layer as big as its plan rather than as big as the target, and that
//! one change gave the frame two coordinate spaces — device space, which the encoder
//! and the damage list speak, and an attachment's own space, which a scissor and a
//! shader's origin speak. Every conversion between them is here, so that the passes can
//! be read as passes: `scissor_in` moves a device rectangle into a region, `meet`
//! answers what a child and its parent share, and `of` is the one place a plan's
//! floating-point bounds become whole pixels.
//!
//! It is `Copy`, it has no device in it, and every one of its answers is checkable by
//! hand — which is why the arithmetic lives apart from the passes that consume it.

/// A plan's rectangle of the frame: where its texture starts and how big it is.
///
/// Whole pixels, clamped to the target — a layer never holds anything outside the frame,
/// because every lane already draws into `bounds ∩ clip ∩ target` and no further.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    /// The whole target, which is what the root plan renders into.
    pub(crate) const fn whole(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// A device-space rectangle in this region's own coordinates: the two intersected,
    /// then moved to the region's origin.
    ///
    /// What a pass rendering into this region must scissor to when the frame patches
    /// damage (ADR 0012): the damage bounding box is stated in device space and a
    /// scissor is stated in the attachment's, and wgpu refuses — with a validation
    /// error, not a wrong picture — a scissor that leaves the attachment. Empty when the
    /// two do not meet, which draws nothing; the pass's load op still clears, so the
    /// attachment is written whole either way (`layers.rs` relies on that for reuse).
    pub(crate) fn scissor_in(self, rect: [u32; 4]) -> [u32; 4] {
        let [x, y, width, height] = rect;
        let left = x.max(self.x);
        let top = y.max(self.y);
        let right = x
            .saturating_add(width)
            .min(self.x.saturating_add(self.width));
        let bottom = y
            .saturating_add(height)
            .min(self.y.saturating_add(self.height));
        [
            left.saturating_sub(self.x),
            top.saturating_sub(self.y),
            right.saturating_sub(left),
            bottom.saturating_sub(top),
        ]
    }

    /// This region as a device-space rectangle.
    pub(crate) const fn rect(self) -> [u32; 4] {
        [self.x, self.y, self.width, self.height]
    }

    /// The rectangle two regions share, or `None` when they do not meet.
    ///
    /// A child's region and its parent's need not contain one another: a plan's bounds
    /// grow by each child's bounds **intersected with the clip the composite will apply**
    /// (`encode.rs`), so a child clipped down to a corner has a region larger than the
    /// part of the parent it can reach. Their meeting is what the composite writes.
    ///
    /// `None` is a child with no part of its parent to write, which composites to
    /// nothing. The encoder emits no child that its clip empties (ADR 0041), so what is
    /// left here is what the two roundings can produce on their own — this rounds pixel
    /// regions out and clamps them to the target, where the encoder tests device-space
    /// rectangles — and the answer the arithmetic gives for it is the same one:
    /// `clip_coverage` is zero everywhere such a child could have contributed.
    pub(crate) fn meet(self, other: Self) -> Option<Self> {
        let [x, y, width, height] = self.scissor_in(other.rect());
        (width > 0 && height > 0).then_some(Self {
            x: self.x.saturating_add(x),
            y: self.y.saturating_add(y),
            width,
            height,
        })
    }

    /// The pixels a plan's device bounds cover, rounded out and clamped.
    ///
    /// A plan that marks nothing gets one texel rather than none: wgpu refuses a
    /// zero-sized texture, and a composite still reads whatever the plan left — which
    /// for an empty plan is a cleared texel that contributes nothing.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped below
    pub(crate) fn of(bounds: Option<[f32; 4]>, width: u32, height: u32) -> Self {
        let Some([x0, y0, x1, y1]) = bounds else {
            return Self {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            };
        };
        let left = x0.floor().max(0.0).min(f32::from(u16::MAX)) as u32;
        let top = y0.floor().max(0.0).min(f32::from(u16::MAX)) as u32;
        let right = (x1.ceil().max(0.0).min(f32::from(u16::MAX)) as u32).min(width);
        let bottom = (y1.ceil().max(0.0).min(f32::from(u16::MAX)) as u32).min(height);
        Self {
            x: left.min(width.saturating_sub(1)),
            y: top.min(height.saturating_sub(1)),
            width: right.saturating_sub(left).max(1),
            height: bottom.saturating_sub(top).max(1),
        }
    }
}

/// The rectangle two scissor rectangles share, in the coordinates both are stated in.
pub(super) fn overlap(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    let left = a[0].max(b[0]);
    let top = a[1].max(b[1]);
    let right = a[0].saturating_add(a[2]).min(b[0].saturating_add(b[2]));
    let bottom = a[1].saturating_add(a[3]).min(b[1].saturating_add(b[3]));
    [
        left,
        top,
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    ]
}
