//! Colours, paints and stroke parameters.
//!
//! # The contract
//!
//! - **Colours reaching us are already device RGB.** Colour management happens upstream:
//!   ICC profiles, `DeviceCMYK`, `CalRGB`, `Separation`, rendering intents, black point
//!   compensation. `ColourSpace::to_rgb` in the caller's tree is the only place a colour
//!   becomes RGB and adding a second one is forbidden. §3 is blunt about the consequence:
//!   *if we offer colour management it will not be used, and if it is on by default the
//!   library cannot be used at all.*
//! - **Straight alpha at the boundary, premultiplied internally.** Converting once at the
//!   boundary is cheaper than converting per comparison, and it is what PNG and the
//!   caller's harness expect. [`Color`] is the boundary form; premultiplication is the
//!   device's business and happens on the device's side of the line.
//! - **Stroke widths are given, not derived.** ISO 32000-2 §8.4.3.2 with §10.7.5 makes a
//!   `0 w` line one device pixel, and the caller resolves that into its
//!   `Stroke::device_width` before we see it. `tiny-skia` happened to do the right
//!   thing, so the rule went unwritten and **every zero-width line was invisible on the
//!   GPU for fifteen sessions**. We take the width we are given, which is therefore
//!   always positive.
//! - **Dashing is already done.** The caller dashes its own paths, including zero-length
//!   dashes whose caps face along the path — Skia's dasher loses that direction and paints
//!   them upright, and on a diagonal dotted line the two answers cover different pixels.
//!   What we must not do is undo it, so [`Stroke`] has no dash array at all, and that
//!   absence is load-bearing.
//! - **Degenerate subpaths are already split.** §8.5.3.2 makes a zero-length subpath a dot
//!   under round caps and *nothing* under butt or square; three libraries gave three
//!   answers and none was the standard's. We draw what we are given.
//!
//! # State
//!
//! Complete as of M7: solid colours, the axial and radial shadings of ISO 32000-2
//! §8.7.4.5.2/.3 over an uploaded ramp, and the caller's pre-rasterised meshes.
//!
//! [`Paint::Function`] is the exception to the module's first contract line, and
//! deliberately: it is the one paint whose colour is *not* resolved upstream, because
//! resolving it upstream is what costs the caller 1 142.8 ms of scene building on a page
//! whose whole content is one §8.7.4.5.2 type 1 shading. What travels is a handle to a
//! compiled §7.10.5 program, not a sampled grid; ADR 0053 is the decision and
//! [`crate::function`] the vocabulary.

use crate::scene::MAX_COORDINATE;

/// A colour in device RGB with straight (non-premultiplied) alpha, each component
/// nominally in `0..=1`.
///
/// Device RGB because colour management happened upstream and must not happen again
/// (§3 of the brief); straight alpha because that is the boundary convention — the
/// device premultiplies once, internally, on its own side of the line.
///
/// The fields are public and unvalidated: a `Color` is a plain value, and the range and
/// finiteness checks live at the scene boundary ([`crate::scene::SceneBuilder`]), which
/// is where §4.7's loud refusal belongs.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Color {
    /// Red, `0..=1`.
    pub r: f32,
    /// Green, `0..=1`.
    pub g: f32,
    /// Blue, `0..=1`.
    pub b: f32,
    /// Straight alpha, `0..=1`; `1` is opaque.
    pub a: f32,
}

impl Color {
    /// A colour from its four components.
    #[must_use]
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Whether every component is finite and within `0..=1`.
    ///
    /// This is the validity test the scene boundary applies; it lives here so that the
    /// definition of a valid colour is written exactly once.
    #[must_use]
    pub fn is_valid(self) -> bool {
        let in_range = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
        in_range(self.r) && in_range(self.g) && in_range(self.b) && in_range(self.a)
    }
}

/// The geometry of a ramp-based shading, in the shading's **own** coordinate space —
/// [`Paint::Shading`]'s `transform` carries it into the scene's space, and the
/// viewport carries the scene to the device (a scene stays viewport-free, §2.3).
///
/// The shading space is deliberately independent of the shaded command's transform:
/// ISO 32000-2 §8.7.4.3's shading matrix anchors a shading to the page, not to the
/// path being filled, so two paths filled with one shading sweep continuously across
/// both.
///
/// The decision deferred from M2 — geometry on the paint versus resolved at upload —
/// lands on the paint: a ramp uploaded once serves any number of placements, which
/// is the §2.2 economy, and the geometry is six floats plus the transform
/// (integration note 9 records it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShadingKind {
    /// ISO 32000-2 §8.7.4.5.2 (PDF type 2): colour varies along a line.
    Axial {
        /// Where the ramp's first colour sits.
        start: crate::geom::Point,
        /// Where its last colour sits.
        end: crate::geom::Point,
        /// Whether the shading continues beyond each end. Where it does not,
        /// nothing is painted there at all — a band, not a wash.
        extend: (bool, bool),
    },
    /// §8.7.4.5.3 (PDF type 3): colour varies between two circles.
    Radial {
        /// Centre of the circle carrying the ramp's first colour.
        start: crate::geom::Point,
        /// Its radius.
        start_radius: f32,
        /// Centre of the circle carrying the ramp's last colour.
        end: crate::geom::Point,
        /// Its radius.
        end_radius: f32,
        /// Whether the shading continues beyond each circle.
        extend: (bool, bool),
    },
}

impl ShadingKind {
    /// Whether every number is finite and within the scene's coordinate bound, and
    /// radii are non-negative.
    #[must_use]
    pub fn is_valid(self) -> bool {
        let ok = |v: f32| v.is_finite() && v.abs() <= MAX_COORDINATE;
        match self {
            Self::Axial { start, end, .. } => ok(start.x) && ok(start.y) && ok(end.x) && ok(end.y),
            Self::Radial {
                start,
                start_radius,
                end,
                end_radius,
                ..
            } => {
                ok(start.x)
                    && ok(start.y)
                    && ok(end.x)
                    && ok(end.y)
                    && ok(start_radius)
                    && ok(end_radius)
                    && start_radius >= 0.0
                    && end_radius >= 0.0
            }
        }
    }
}

/// How a fill or stroke is painted.
///
/// Every heavy paint is a resource identifier plus the geometry that places it, and
/// [`Paint::Function`] is deliberately the same shape as [`Paint::Shading`]: the program
/// is uploaded once (§2.2's economy, and ADR 0053's shader cache keys on it), while the
/// domain, matrix, range and background belong to the *shading* — two shadings may share
/// one program under different matrices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Paint {
    /// A single uniform colour.
    Solid(Color),
    /// §8.7.4.5.2/.3: a ramp uploaded once, swept by the kind's geometry.
    Shading {
        /// The uploaded colour ramp.
        ramp: crate::ids::RampId,
        /// The sweep, in the shading's own space.
        kind: ShadingKind,
        /// Shading space → scene space (§8.7.4.3's shading matrix). Deliberately
        /// **not** the command's transform: a shading is anchored to the page, so
        /// the same paint sweeps continuously across differently-placed shapes.
        transform: crate::geom::Affine,
    },
    /// §8.7.4.5.5–.7, pre-rasterised by the caller and shared between its backends
    /// (integration note 5): sampled at absolute device pixels.
    Mesh(crate::ids::MeshId),
    /// §8.7.4.5.2's type 1 shading: a colour the device evaluates per fragment
    /// (ADR 0053).
    ///
    /// # What a device does with it, in order
    ///
    /// 1. Map the fragment's position back through `matrix` into the shading's own space.
    ///    That the matrix is invertible is checked at the scene boundary, not there.
    /// 2. Decide whether that position is inside `domain`. §8.7.4.5.2 is explicit that the
    ///    domain rectangle is a *region*, not a clamp on the position:
    ///
    ///    > Points within the shading's bounding box (`BBox`) that fall outside this
    ///    > transformed domain rectangle shall be painted with the shading's background
    ///    > colour (`Background`); if the shading dictionary has no `Background` entry,
    ///    > such points shall be left unpainted.
    ///
    ///    So outside the domain the device emits `background`, or — when it is `None` —
    ///    **nothing at all**: alpha zero, no coverage. Never the nearest edge's colour;
    ///    that is the plausible wrong page §5 has a name for.
    /// 3. Inside it, run the program and clip each output into its `range` pair. That
    ///    clip is §7.10.1's, and it is not optional:
    ///
    ///    > Input values passed to the function shall be clipped to the domain, and output
    ///    > values produced by the function shall be clipped to the range.
    ///
    ///    The two clauses do not conflict: §7.10.1 governs an invocation of the function,
    ///    §8.7.4.5.2 governs where a type 1 shading invokes it.
    Function {
        /// The uploaded program.
        program: crate::ids::FunctionId,
        /// §8.7.4.5.2's `Domain`, in the shading's own space. A [`Rect`](crate::geom::Rect)
        /// rather than four floats, so the ordering convention is enforced by the type
        /// rather than documented beside it.
        domain: crate::geom::Rect,
        /// §8.7.4.5.2's `Matrix`: shading space → scene space. Deliberately **not** the
        /// command's transform, for the reason [`Paint::Shading`]'s `transform` states.
        matrix: crate::geom::Affine,
        /// §7.10.1's `Range`, and the component count with it.
        range: crate::function::FnRange,
        /// §8.7.4.5.2's `Background`, resolved to a device colour upstream. `None` leaves
        /// points outside the domain rectangle unpainted, which is what the clause says
        /// happens when a shading dictionary has no `Background` entry.
        ///
        /// Table 77 restricts the entry further — "The background colour shall be applied
        /// only when the shading is used as part of a shading pattern, not when painted
        /// directly with the `sh` operator" — and that distinction is one only the caller
        /// can see, so it arrives already applied: `None` from an `sh`, whatever the
        /// pattern declared otherwise.
        background: Option<Color>,
    },
}

impl Paint {
    /// Whether the paint's values are valid at the scene boundary.
    ///
    /// The [`Paint::Function`] arm answers about the *paint* — its domain, matrix, range
    /// and background. Whether the program it names is one a device can execute is asked
    /// at upload instead ([`check_program`](crate::function::check_program)), because a
    /// program is a resource and a scene holds only its identifier.
    #[must_use]
    pub fn is_valid(self) -> bool {
        match self {
            Self::Solid(color) => color.is_valid(),
            Self::Shading {
                kind, transform, ..
            } => kind.is_valid() && transform.is_finite(),
            Self::Mesh(_) => true,
            // The rule is written once, where the boundary applies it, so that the
            // predicate and the named refusal cannot drift apart.
            Self::Function {
                domain,
                matrix,
                range,
                background,
                ..
            } => crate::scene::validate::function::check_function_paint(
                domain, matrix, range, background,
            )
            .is_ok(),
        }
    }
}

/// One stop of a colour ramp, for the axial, radial and function-based shadings of
/// ISO 32000-2 §8.7.4.5 (drawn from M7; validated at upload from M2).
///
/// Stops reach `Device::upload_ramp` in ascending `offset` order spanning `0..=1`,
/// matching the caller's own `Ramp` — colour and position resolved upstream so the two
/// backends cannot disagree about them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stop {
    /// Position along the ramp, `0..=1`, ascending across the slice.
    pub offset: f32,
    /// The colour at this position.
    pub color: Color,
}

/// Treatment of open subpath ends, ISO 32000-2 §8.4.3.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineCap {
    /// Terminates exactly at the endpoint (line cap style 0).
    #[default]
    Butt,
    /// A semicircle of diameter equal to the line width (style 1).
    Round,
    /// A square extending half the line width past the endpoint (style 2).
    Square,
}

/// Treatment of segment joins, ISO 32000-2 §8.4.3.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineJoin {
    /// Outer edges extended to meet, subject to the miter limit (line join style 0).
    #[default]
    Miter,
    /// An arc of diameter equal to the line width (style 1).
    Round,
    /// The outer corner cut off with a straight edge (style 2).
    Bevel,
}

/// Parameters of a stroke, with the decisions that are the caller's already taken
/// (§4.5 of the brief): the width is resolved (a PDF `0 w` arrived here as one device
/// pixel's width), dashing already happened, and degenerate subpaths were pre-split.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stroke {
    /// The width in the **command's own space** — the space the outline's coordinates
    /// are stated in — resolved to a device width by the encode, per placement
    /// (ADR 0085; the caller's contract amendment of 2026-08-27).
    ///
    /// Zero is a legitimate width and means what ISO 32000-2 §8.4.3.2 says of it:
    ///
    /// > A line width of 0 shall denote the thinnest line that can be rendered at
    /// > device resolution: 1 device pixel wide.
    ///
    /// [`Stroke::adjust`] carries §10.7.5's automatic stroke adjustment. Both rules
    /// are applied where the composed device transform is known — the encode — which
    /// is what lets one scene state one stroke and be true at every magnification.
    pub width: f32,
    /// ISO 32000-2 §10.7.5's automatic stroke adjustment: when set, a stroke whose
    /// device width would fall below half a pixel is drawn one device pixel wide.
    pub adjust: bool,
    /// Treatment of open subpath ends.
    pub cap: LineCap,
    /// Treatment of segment joins.
    pub join: LineJoin,
    /// Ratio at which a miter join becomes a bevel, ISO 32000-2 §8.4.3.5. At least 1.
    pub miter_limit: f32,
    // No dash array, and that absence is load-bearing: see the module docs.
}

impl Stroke {
    /// Whether the stroke's numbers are valid at the scene boundary: a finite,
    /// non-negative width no larger than [`MAX_COORDINATE`] — zero is §8.4.3.2's
    /// thinnest-line request, resolved at encode (ADR 0085) — and a finite miter
    /// limit of at least 1 (ISO 32000-2 §8.4.3.5 defines the limit as a ratio ≥ 1).
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.width.is_finite()
            && self.width >= 0.0
            && self.width <= MAX_COORDINATE
            && self.miter_limit.is_finite()
            && self.miter_limit >= 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::{Color, LineCap, LineJoin, Stroke};

    /// The boundary's definition of a valid colour: finite, and inside the unit range
    /// on every component. Out-of-range and NaN are refused, not clamped — clamping
    /// would be a silent repair of data §4.7 says to refuse loudly.
    #[test]
    fn validity_is_finite_and_in_unit_range() {
        assert!(Color::new(0.0, 0.5, 1.0, 1.0).is_valid());
        assert!(!Color::new(-0.01, 0.0, 0.0, 1.0).is_valid());
        assert!(!Color::new(0.0, 1.01, 0.0, 1.0).is_valid());
        assert!(!Color::new(0.0, 0.0, f32::NAN, 1.0).is_valid());
        assert!(!Color::new(0.0, 0.0, 0.0, f32::INFINITY).is_valid());
    }

    /// A stroke's width is scene-space and non-negative since ADR 0085 — zero is
    /// §8.4.3.2's thinnest-line request, resolved at encode — and its miter limit is a
    /// ratio of at least one, per §8.4.3.5.
    #[test]
    fn stroke_validity_pins_the_resolved_width_contract() {
        let valid = Stroke {
            width: 1.5,
            adjust: false,
            cap: LineCap::Round,
            join: LineJoin::Miter,
            miter_limit: 10.0,
        };
        assert!(valid.is_valid());
        assert!(
            Stroke {
                width: 0.0,
                adjust: false,
                ..valid
            }
            .is_valid(),
            "zero means the thinnest line, and the encode resolves it (ADR 0085)"
        );
        assert!(
            !Stroke {
                width: -0.5,
                adjust: false,
                ..valid
            }
            .is_valid(),
            "a negative width is not a width (§8.4.3.2's range)"
        );
        assert!(
            !Stroke {
                width: f32::NAN,
                adjust: false,
                ..valid
            }
            .is_valid()
        );
        assert!(
            !Stroke {
                miter_limit: 0.5,
                ..valid
            }
            .is_valid()
        );
    }
}
