//! A realised soft mask: where it sits, and what it is everywhere else.
//!
//! The reduction itself is `shaders/reduce.wgsl` — the scene-side vocabulary is
//! [`quorra_scene::mask`], and ISO 32000-2 §11.5 is the clause. This module is the
//! placement that reduction produces: a mask is a texture over part of the frame, and
//! five shaders sample it in device space.
//!
//! [`quorra_scene::mask`]: ../quorra_scene/mask/index.html
//!
//! # Why a mask is not simply a full-target texture
//!
//! It was one until ADR 0037. §11.5 makes a soft mask *a transparency group rendered at
//! device resolution*, and a group covers what it draws: on `issue16287.pdf` at 4× the
//! four masks of a 2 448 × 9 504 page cost 93 MB realised whole, against a 268 MiB frame
//! budget the page then exceeded. So a mask is realised at its own plan's rectangle, like
//! every other layer.
//!
//! # What a sample outside that rectangle is
//!
//! **Not zero, and not a clamp to the edge.** The mask's group marks nothing outside its
//! bounds, so what a full-target realisation held there is exactly what the reduce writes
//! for a *fully transparent* pixel — `transfer[0]` under §11.5.2's alpha rule, and the
//! transferred luminosity of §11.5.3's backdrop under the luminosity one, which is why a
//! luminosity mask with a white backdrop admits everything outside its group and one with
//! the caller's default black admits nothing. [`MaskPlacement::outside`] carries that
//! constant, and a mask that is absent altogether is the placement with no area at all,
//! whose every sample is therefore its `outside` of 1.
//!
//! [`transparent_value`] is a second implementation of `reduce.wgsl`'s own arithmetic for
//! the transparent case, on the CPU because five uniforms need the number before any pass
//! runs. Two implementations of one rule is what the reduction already is (that one
//! against the caller's `SoftMask::value`, §4.2), and it is held the same way: by a test
//! that renders an empty mask group on the device and compares the texel to this
//! function, over both rules and a spread of backdrops and tables.

use crate::encode::MaskPlan;

/// One mask the frame has reduced: the R8 texture, and where it is.
///
/// The two travel together because every sampler of a mask needs both, and a view bound
/// beside somebody else's placement reads the right texels at the wrong coordinates.
#[derive(Debug)]
pub(crate) struct Realised {
    /// The reduced R8 mask.
    pub view: wgpu::TextureView,
    /// Where those texels are, and what surrounds them.
    pub placement: MaskPlacement,
}

/// Where a realised mask sits in device space, and what it holds outside that.
///
/// The five shaders that sample a mask carry these three numbers, and each states in its
/// own comment what it does with them — ADR 0028's discipline, learned from a pane that
/// taught three places to subtract an origin and shipped with one of them missing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MaskPlacement {
    /// The device pixel the mask texture's texel (0, 0) is.
    pub origin: [f32; 2],
    /// The mask texture's size in texels. `[0.0, 0.0]` for a mask that does not exist,
    /// whose every sample falls outside it.
    pub size: [f32; 2],
    /// The mask's value outside `size` texels from `origin`: the reduce's own output for
    /// a fully transparent pixel, since that is what the mask's group leaves there.
    pub outside: f32,
}

impl MaskPlacement {
    /// No mask at all: no area, and everything admitted (§11.5's default state, where a
    /// scene that names no mask is masked by nothing).
    ///
    /// Stated as a placement rather than as the 1 × 1 white texture a clamp used to read,
    /// because "outside every mask rectangle" is now a case the shaders have, and an
    /// absent mask is the whole of it.
    pub(crate) const ABSENT: Self = Self {
        origin: [0.0, 0.0],
        size: [0.0, 0.0],
        outside: 1.0,
    };

    /// The 32 uniform bytes the shaders read: `rect` as (origin.xy, size.xy), then
    /// `outside` in the next slot's `.x`. One writer, so the five layouts cannot drift.
    pub(crate) fn bytes(self) -> [u8; 32] {
        let mut bytes = [0_u8; 32];
        let values = [
            self.origin[0],
            self.origin[1],
            self.size[0],
            self.size[1],
            self.outside,
        ];
        for (slot, value) in bytes.chunks_exact_mut(4).zip(values) {
            slot.copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

/// What `reduce.wgsl` writes where the mask's group is fully transparent.
///
/// The shader's own `a_byte == 0` path, in its operation order: §11.5.2's alpha rule
/// derives 0 there, §11.5.3's luminosity rule derives the luminosity of its opaque
/// backdrop (the clause's own 0.30 / 0.59 / 0.11 coefficients, the group contributing
/// nothing), and §11.6.5.1's transfer table then maps whichever byte results.
///
/// Returned as the float an R8 texel would read back as: the shader stores
/// `f32(value) / 255`, which lands exactly on the unorm level, so the two are the same
/// number rather than merely close.
pub(crate) fn transparent_value(plan: &MaskPlan) -> f32 {
    let derived = if plan.kind_word == 0 {
        0
    } else {
        let [r, g, b] = plan.backdrop;
        let luminosity = 0.30_f32.mul_add(r, 0.59_f32.mul_add(g, 0.11 * b));
        // Clamped into 0..=255 on the line above the cast, which is what makes it exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            (luminosity * 255.0).round().clamp(0.0, 255.0) as u8
        }
    };
    f32::from(plan.table[usize::from(derived)]) / 255.0
}
