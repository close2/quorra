//! How a mark combines with what is already there.
//!
//! Three enumerations, all of them transcriptions of a specification rather than design
//! choices of ours, which is why they are real code in a skeleton (`doc/adr/0003`):
//! [`BlendMode`] because ISO 32000-2 §11.3.5 names sixteen modes, [`Compose`] because
//! §11.4.6 needs a second compositing behaviour that a general vector API does not have,
//! and [`FillRule`] because §8.5.3.3 defines two.
//!
//! The functions that *implement* these arrive with M6 and are ours alone: the caller's
//! CPU backend implements the sixteen modes itself rather than using `tiny-skia`'s,
//! because three of `tiny-skia`'s were wrong — one by 113 of 255 — and because sharing an
//! implementation between the two backends would make the cross-backend comparison
//! compare one implementation with itself.

/// One of ISO 32000-2 §11.3.5's sixteen blend modes.
///
/// The clause divides them into the twelve *separable* modes of §11.3.5.2, each defined
/// by a function applied to one colour component at a time, and the four *non-separable*
/// modes of §11.3.5.3, defined by the clause's `Lum`, `ClipColor`, `SetLum` and `SetSat`
/// functions over all three components at once. No per-component formula produces the
/// non-separable four, and a backend that gets one subtly wrong still produces a
/// plausible picture — which is why [`Self::is_separable`] exists and why the
/// sixteen-mode conformance scene is part of M6 rather than a later tidy-up.
///
/// PDF's deprecated `/Compatible` name, which means `Normal`, is resolved by the caller;
/// sixteen is what reaches us.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlendMode {
    /// §11.3.5.2. The backdrop is not used: the result is the source.
    ///
    /// The initial value of the blend mode in §11.6.6's graphics state, and what the
    /// overwhelming majority of commands on a page carry. Fourteen cross-backend
    /// fixtures in the caller's tree carried nothing else, which is how sixteen blend
    /// functions came to have never been compared at all.
    #[default]
    Normal,
    /// §11.3.5.2. Multiplies the backdrop and source; the result is at least as dark as
    /// either.
    Multiply,
    /// §11.3.5.2. Multiplies the complements; the result is at least as light as either.
    Screen,
    /// §11.3.5.2. `Multiply` or `Screen` depending on the backdrop, so the backdrop
    /// decides the contrast.
    Overlay,
    /// §11.3.5.2. The darker of backdrop and source, per component.
    Darken,
    /// §11.3.5.2. The lighter of backdrop and source, per component.
    Lighten,
    /// §11.3.5.2. Brightens the backdrop in proportion to the source.
    ColorDodge,
    /// §11.3.5.2. Darkens the backdrop in proportion to the source.
    ColorBurn,
    /// §11.3.5.2. `Multiply` or `Screen` depending on the source — `Overlay` with the
    /// roles exchanged.
    HardLight,
    /// §11.3.5.2. Darkens or lightens depending on the source, with the clause's own
    /// auxiliary function `D` in the middle band.
    SoftLight,
    /// §11.3.5.2. The absolute difference of backdrop and source.
    Difference,
    /// §11.3.5.2. Excludes rather than differences: light where exactly one is light.
    Exclusion,
    /// §11.3.5.3, non-separable. The source's hue with the backdrop's saturation and
    /// luminosity.
    Hue,
    /// §11.3.5.3, non-separable. The source's saturation with the backdrop's hue and
    /// luminosity.
    Saturation,
    /// §11.3.5.3, non-separable. The source's hue and saturation with the backdrop's
    /// luminosity.
    Color,
    /// §11.3.5.3, non-separable. The source's luminosity with the backdrop's hue and
    /// saturation.
    Luminosity,
}

impl BlendMode {
    /// Every mode ISO 32000-2 §11.3.5 defines, in the clause's own order.
    ///
    /// Exhaustive by construction rather than by good intentions: the conformance scene
    /// of §4.3 is generated from this array, so a mode that exists cannot be a mode the
    /// suite does not draw.
    pub const ALL: [Self; 16] = [
        Self::Normal,
        Self::Multiply,
        Self::Screen,
        Self::Overlay,
        Self::Darken,
        Self::Lighten,
        Self::ColorDodge,
        Self::ColorBurn,
        Self::HardLight,
        Self::SoftLight,
        Self::Difference,
        Self::Exclusion,
        Self::Hue,
        Self::Saturation,
        Self::Color,
        Self::Luminosity,
    ];

    /// Whether this mode is one of §11.3.5.2's separable twelve, which act on each
    /// colour component independently.
    ///
    /// The four for which this is false are §11.3.5.3's, and they are the ones a shader
    /// cannot express as a per-component formula.
    #[must_use]
    pub const fn is_separable(self) -> bool {
        !matches!(
            self,
            Self::Hue | Self::Saturation | Self::Color | Self::Luminosity
        )
    }
}

/// Which Porter-Duff compositing operator a mark uses.
///
/// This enumeration is the reason a general 2D vector library cannot be patched into
/// ISO 32000-2 clause 11, and `doc/RENDER_LIBRARY.md` §4.1 is the argument. §11.4.6:
///
/// > In a knockout group, each individual element shall be composited with the group's
/// > initial backdrop rather than with the stack of preceding elements in the group.
///
/// The initial backdrop is transparent, so compositing an element with it yields the
/// element; the group's accumulated result is then replaced by *a fraction* of that, and
/// the fraction is the element's **shape**. For a rasteriser, shape is the coverage the
/// element was drawn with — and a raster of premultiplied samples carries opacity, not
/// shape. Vello's layers composite over the layer's whole *bounding box*, so its
/// `Compose::Copy` erased a row of pixels outside the shape entirely, and no arrangement
/// of an SVG-shaped API recovers the difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Compose {
    /// Porter-Duff Source-over: the source over the backdrop, weighted by coverage.
    ///
    /// Every ordinary mark on a page, and the initial state.
    #[default]
    SrcOver,
    /// Porter-Duff Source, **modulated by coverage**: an element with 40% coverage
    /// replaces 40% of what was there and leaves the rest.
    ///
    /// §11.4.6's knockout groups, applied per element. The modulation is the whole
    /// point: a plain "replace the destination" operator is the same thing wherever
    /// coverage is 1 and wrong everywhere else, which is why the scene that tests this
    /// has a diagonal edge on purpose — a scene of axis-aligned rectangles would agree
    /// while being wrong.
    Src,
}

/// Which of ISO 32000-2 §8.5.3.3's two rules decides the inside of a path.
///
/// Both appear on real pages, including the case that catches an implementation out: a
/// nested subpath wound in the *same* direction as its parent, where the two rules
/// disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FillRule {
    /// §8.5.3.3.2, the nonzero winding number rule. The `f` operator's rule, and the
    /// default.
    #[default]
    NonZero,
    /// §8.5.3.3.3, the even-odd rule. The `f*` operator's.
    EvenOdd,
}

#[cfg(test)]
mod tests {
    use super::{BlendMode, Compose, FillRule};

    /// ISO 32000-2 §11.3.5 names sixteen modes; [`BlendMode::ALL`] is what the
    /// conformance scene is generated from, so a missing entry would silently shrink the
    /// suite rather than fail a build.
    #[test]
    fn all_holds_sixteen_distinct_modes() {
        let mut seen = BlendMode::ALL.to_vec();
        seen.sort_by_key(|mode| format!("{mode:?}"));
        seen.dedup();
        assert_eq!(seen.len(), 16, "ALL must list sixteen distinct modes");
    }

    /// §11.3.5.3 defines exactly four non-separable modes. A fifth, or a third, means
    /// the shader that dispatches on this predicate is dispatching on the wrong set.
    #[test]
    fn exactly_four_modes_are_non_separable() {
        let non_separable = BlendMode::ALL
            .iter()
            .filter(|mode| !mode.is_separable())
            .count();
        assert_eq!(non_separable, 4);
    }

    /// §11.6.6 initialises the blend mode to Normal, and an ordinary mark composites
    /// Source-over under the nonzero rule. A `Default` that drifted from the clause
    /// would be wrong in a way no single test of a later stage would localise.
    #[test]
    fn defaults_are_the_initial_graphics_state() {
        assert_eq!(BlendMode::default(), BlendMode::Normal);
        assert_eq!(Compose::default(), Compose::SrcOver);
        assert_eq!(FillRule::default(), FillRule::NonZero);
    }
}
