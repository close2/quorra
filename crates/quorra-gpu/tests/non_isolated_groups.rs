//! ISO 32000-2 §11.4.4's non-isolated group, and the identity that lets one raster
//! draw it.
//!
//! # Where the expected values come from
//!
//! [`clause_group`] transcribes §11.4.4 from the standard — the initialisation, the
//! per-element recurrence, and the Result step that removes the backdrop's
//! contribution — and [`clause_composite`] transcribes §11.3.6's compositing formula.
//! Both are written from the clause, independently of `composite.wgsl` and of the
//! caller's backend: two transcriptions of one clause, checked against each other
//! (principle 5).
//!
//! The first test is the whole argument for ADR 0019. §11.4.4's removal divides by
//! Table 140's group alpha, which a premultiplied raster does not hold, and NOTE 4
//! advises keeping a second set of accumulators for it. It is not needed, because the
//! quantity divided out is multiplied straight back in by the composite that follows —
//! but only under the **Normal** blend function, and only outside a knockout group.
//! The test measures all three: the identity where it holds, and the two negative
//! controls that say what the builder's refusals are for. Neither control is decoration
//! — an implementation that quietly dropped the conditions would pass every other test
//! in this file.

// Test-file lint policy as in m1.rs; the reference math mirrors clause arithmetic.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use quorra_scene::{
    Affine, BlendMode, Color, Compose, GroupSpec, MaskKind, NonIsolatedReason, Point, Rect, Scene,
    SceneBuilder, SceneError,
};

mod common;

use common::headless::{device, render};
use common::probe::pixel;

// ---------------------------------------------------------------- the clause itself

/// §11.3.1's Union(b, s) = b + s − b·s, the "union" of two alphas.
fn union(b: f64, s: f64) -> f64 {
    b + s - b * s
}

/// §11.3.5's B(Cb, Cs) for the modes this file uses. (m6.rs holds all sixteen against
/// the same clause; repeating them here would be a second place to get them wrong.)
fn blend(mode: BlendMode, cb: f64, cs: f64) -> f64 {
    match mode {
        BlendMode::Normal => cs,
        BlendMode::Multiply => cb * cs,
        BlendMode::Screen => cb + cs - cb * cs,
        BlendMode::Difference => (cb - cs).abs(),
        other => panic!(
            "this file's reference covers Normal, Multiply, Screen, Difference, not {other:?}"
        ),
    }
}

/// One group element: a straight colour, a source alpha (shape × opacity — §11.3.6's
/// NOTE 1 works in alpha, and a raster conflates shape with it), and a blend mode.
#[derive(Clone, Copy, Debug)]
struct Element {
    colour: f64,
    alpha: f64,
    mode: BlendMode,
}

/// §11.4.4's group compositing function for a non-isolated, non-knockout group, and
/// §11.4.5's one-line alteration for an isolated one (`a0 = 0.0`).
///
/// Returns the group's computed colour `C` and alpha `α = αgn` — Table 139's results,
/// which are then used as the source colour and object alpha when the group is
/// composited with its backdrop.
fn clause_group(c0: f64, a0: f64, elements: &[Element], isolated: bool) -> (f64, f64) {
    let backdrop_alpha = if isolated { 0.0 } else { a0 };
    // Initialization: f_g0 = alpha_g0 = 0.0.
    let mut ag = 0.0_f64;
    let mut c_prev = c0;
    let mut a_prev = backdrop_alpha;
    for e in elements {
        // alpha_gi = Union(alpha_g(i-1), alpha_si); alpha_i = Union(alpha_0, alpha_gi).
        ag = union(ag, e.alpha);
        let a_i = union(backdrop_alpha, ag);
        // Ci = (1 − αsi/αi)·C(i−1) + (αsi/αi)·((1 − α(i−1))·Csi + α(i−1)·Bi(C(i−1), Csi))
        let mixed = (1.0 - a_prev) * e.colour + a_prev * blend(e.mode, c_prev, e.colour);
        c_prev = if a_i <= 0.0 {
            0.0
        } else {
            (1.0 - e.alpha / a_i) * c_prev + (e.alpha / a_i) * mixed
        };
        a_prev = a_i;
    }
    // Result: C = Cn + (Cn − C0)·(α0/αgn − α0). For an isolated group NOTE 2 says this
    // simplifies to C = Cn, there being no backdrop contribution to factor out.
    let c = if isolated || ag <= 0.0 {
        c_prev
    } else {
        c_prev + (c_prev - c0) * (backdrop_alpha / ag - backdrop_alpha)
    };
    (c, ag)
}

/// §11.3.6's compositing formula for one object over a backdrop: straight colours in,
/// straight colour and alpha out. `w` is the constant alpha applied to the object.
fn clause_composite(
    cb: f64,
    ab: f64,
    cs: f64,
    object_alpha: f64,
    w: f64,
    mode: BlendMode,
) -> (f64, f64) {
    let as_ = object_alpha * w;
    let ar = union(ab, as_);
    if ar <= 0.0 {
        return (0.0, 0.0);
    }
    let cr = (1.0 - as_ / ar) * cb + (as_ / ar) * ((1.0 - ab) * cs + ab * blend(mode, cb, cs));
    (cr, ar)
}

/// What quorra does instead: seed the group's buffer with the backdrop, draw the
/// elements onto it with the ordinary per-element compositing, then interpolate.
/// Premultiplied throughout, which is what a raster holds.
fn seeded_and_interpolated(c0: f64, a0: f64, elements: &[Element], w: f64) -> (f64, f64) {
    let (mut cb, mut ab) = (c0, a0);
    for e in elements {
        let (c, a) = clause_composite(cb, ab, e.colour, e.alpha, 1.0, e.mode);
        cb = c;
        ab = a;
    }
    // result = (1 − w)·B + w·E(B)
    (
        (1.0 - w) * (a0 * c0) + w * (ab * cb),
        (1.0 - w) * a0 + w * ab,
    )
}

/// A fixed generator: this is a proof obligation, not a fuzz run, so the same inputs
/// are examined on every machine and every run.
struct Fixed(u64);

impl Fixed {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 11) as f64) / ((1_u64 << 53) as f64)
    }

    fn mode(&mut self) -> BlendMode {
        match (self.next() * 4.0) as u32 {
            0 => BlendMode::Normal,
            1 => BlendMode::Multiply,
            2 => BlendMode::Screen,
            _ => BlendMode::Difference,
        }
    }

    fn elements(&mut self) -> Vec<Element> {
        let n = 1 + (self.next() * 4.0) as usize;
        (0..n)
            .map(|_| Element {
                colour: self.next(),
                alpha: self.next(),
                mode: self.mode(),
            })
            .collect()
    }
}

/// The worst deviation, over `TRIALS` random configurations, between the clause run in
/// full and the seed-and-interpolate construction — for a group composited under
/// `group_mode`, isolated or not.
fn worst_deviation(group_mode: BlendMode, isolated: bool) -> f64 {
    const TRIALS: usize = 200_000;
    let mut rng = Fixed(0x5eed_1234_9abc_def0);
    let mut worst = 0.0_f64;
    for _ in 0..TRIALS {
        let (c0, a0, w) = (rng.next(), rng.next(), rng.next());
        let elements = rng.elements();
        let (c, ag) = clause_group(c0, a0, &elements, isolated);
        let (cr, ar) = clause_composite(c0, a0, c, ag, w, group_mode);
        let (premul, alpha) = seeded_and_interpolated(c0, a0, &elements, w);
        worst = worst.max((ar * cr - premul).abs()).max((ar - alpha).abs());
    }
    worst
}

/// The identity ADR 0019 rests on, and the two controls that say why the builder
/// refuses what it refuses.
#[test]
fn the_interpolation_is_the_clause_exactly_and_only_under_normal() {
    let held = worst_deviation(BlendMode::Normal, false);
    assert!(
        held < 1e-12,
        "seeding and interpolating must reproduce §11.4.4 exactly; worst deviation {held:e}"
    );

    // Control 1: the cancellation is the Normal composite. Under any other blend the
    // group's own alpha is needed, and this raster does not have it.
    for mode in [
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Difference,
    ] {
        let broken = worst_deviation(mode, false);
        assert!(
            broken > 0.5,
            "a {mode:?} composite must break the identity, not merely bend it: {broken:e}"
        );
    }

    // Control 2: the flag is not decoration — applied to an isolated group the same
    // construction is a different picture, which is what §11.4.5's own NOTE says.
    let wrong_kind = worst_deviation(BlendMode::Normal, true);
    assert!(
        wrong_kind > 0.5,
        "seeding an isolated group must be visibly wrong: {wrong_kind:e}"
    );
}

// ------------------------------------------------------------------ on the device

const W: u32 = 32;
const H: u32 = 16;

fn full_rect() -> Rect {
    Rect::new(Point::new(0.0, 0.0), Point::new(W as f32, H as f32))
}

/// One element that **blends**: §11.3.5 for a single element is an implicit
/// one-element group, and `Command::Rect` always composites Normal, so this is how a
/// scene says "a Multiply rectangle". The clause treats the two identically — for an
/// element that is a group, its computed colour and alpha are the element's.
fn blended_rect(body: &mut SceneBuilder, colour: Color, mode: BlendMode) -> Result<(), SceneError> {
    body.group(
        GroupSpec {
            blend: mode,
            ..group(true, 1.0)
        },
        |inner| inner.rect(full_rect(), Affine::IDENTITY, colour, None, None),
    )
}

fn group(isolated: bool, alpha: f32) -> GroupSpec {
    GroupSpec {
        alpha,
        blend: BlendMode::Normal,
        clip: None,
        knockout: false,
        mask: None,
        isolated,
        compose: Compose::SrcOver,
    }
}

/// A page rectangle, then a group over it holding one blended rectangle.
fn scene_with(isolated: bool, alpha: f32, backdrop: Color, element: Color) -> Scene {
    let mut builder = SceneBuilder::new();
    builder
        .rect(full_rect(), Affine::IDENTITY, backdrop, None, None)
        .unwrap();
    builder
        .group(group(isolated, alpha), |body| {
            blended_rect(body, element, BlendMode::Multiply)
        })
        .unwrap();
    builder.finish()
}

/// The clause's own answer for that scene, as straight-alpha RGBA8.
///
/// Quantisation is applied where the device quantises and nowhere else: colours reach
/// the layer as premultiplied bytes, so the reference works from the same rounded
/// values rather than from the ideal ones (the convention m6.rs established).
fn clause_answer(isolated: bool, alpha: f64, backdrop: Color, element: Color) -> [u8; 4] {
    let quant = |v: f64| (v * 255.0).round() / 255.0;
    let a0 = quant(f64::from(backdrop.a));
    let as_ = quant(f64::from(element.a));
    let mut out = [0_u8; 4];
    let mut ar_out = 0.0_f64;
    for (ch, (b, s)) in [
        (backdrop.r, element.r),
        (backdrop.g, element.g),
        (backdrop.b, element.b),
    ]
    .into_iter()
    .enumerate()
    {
        // Straight colours recovered from the premultiplied bytes the layer holds.
        let c0 = quant(f64::from(b) * f64::from(backdrop.a)) / a0.max(1e-9);
        let cs = quant(f64::from(s) * f64::from(element.a)) / as_.max(1e-9);
        let elements = [Element {
            colour: cs,
            alpha: as_,
            mode: BlendMode::Multiply,
        }];
        let (c, ag) = clause_group(c0, a0, &elements, isolated);
        let (cr, ar) = clause_composite(c0, a0, c, ag, alpha, BlendMode::Normal);
        ar_out = ar;
        let premul = (cr * ar * 255.0).round().clamp(0.0, 255.0) as u32;
        let alpha_byte = (ar * 255.0).round().clamp(0.0, 255.0) as u32;
        // The readback's premultiplied-to-straight conversion, rounded as §3 hands it
        // back; a transparent result has no straight colour to report.
        out[ch] = (premul * 255 + alpha_byte / 2)
            .checked_div(alpha_byte)
            .map_or(0, |v| v.min(255) as u8);
    }
    out[3] = (ar_out * 255.0).round().clamp(0.0, 255.0) as u8;
    out
}

/// The picture §11.4.4 asks for: the element's Multiply sees the page under the group,
/// and the backdrop is counted exactly once in the result.
#[test]
fn a_non_isolated_group_matches_clause_11_4_4() {
    let mut device = device();
    let backdrop = Color::new(0.9, 0.55, 0.2, 1.0);
    let element = Color::new(0.3, 0.7, 0.85, 0.75);

    for alpha in [1.0_f32, 0.5, 0.25] {
        let pixels = render(
            &mut device,
            &scene_with(false, alpha, backdrop, element),
            W,
            H,
        );
        let got = pixel(&pixels, W, W / 2, H / 2);
        let want = clause_answer(false, f64::from(alpha), backdrop, element);
        for ch in 0..4 {
            let diff = (i32::from(got[ch]) - i32::from(want[ch])).abs();
            assert!(
                diff <= 3,
                "alpha {alpha}, channel {ch}: device {} vs §11.4.4 {} (diff {diff})",
                got[ch],
                want[ch]
            );
        }
    }
}

/// The same scene as an isolated group is §11.4.5's picture, and it is a *different*
/// one — the flag reaches the pixels, which no amount of clause-matching on one side
/// alone would prove.
#[test]
fn isolation_changes_the_picture_and_both_match_their_clause() {
    let mut device = device();
    let backdrop = Color::new(0.9, 0.55, 0.2, 1.0);
    let element = Color::new(0.3, 0.7, 0.85, 0.75);

    let isolated = pixel(
        &render(&mut device, &scene_with(true, 1.0, backdrop, element), W, H),
        W,
        W / 2,
        H / 2,
    );
    let non_isolated = pixel(
        &render(
            &mut device,
            &scene_with(false, 1.0, backdrop, element),
            W,
            H,
        ),
        W,
        W / 2,
        H / 2,
    );

    let want_isolated = clause_answer(true, 1.0, backdrop, element);
    for ch in 0..4 {
        let diff = (i32::from(isolated[ch]) - i32::from(want_isolated[ch])).abs();
        assert!(
            diff <= 3,
            "the isolated group must still be §11.4.5's: channel {ch}, device {} vs clause {}",
            isolated[ch],
            want_isolated[ch]
        );
    }

    let apart: i32 = (0..3)
        .map(|ch| (i32::from(isolated[ch]) - i32::from(non_isolated[ch])).abs())
        .max()
        .unwrap();
    assert!(
        apart > 10,
        "isolated {isolated:?} and non-isolated {non_isolated:?} differ by only {apart}; \
         §11.4.4 NOTE 2 says a blending element is exactly where they must not agree"
    );
}

/// The backdrop a non-isolated group is seeded from is **its own**, not the page's:
/// nested inside an isolated group, the element may see what that group drew and
/// nothing under it.
#[test]
fn the_seed_is_the_group_backdrop_not_the_page() {
    let mut device = device();
    // Black under the outer group; if the seed came from the page, multiplying by it
    // would drag the result to black and the assertion below would fail.
    let page = Color::new(0.0, 0.0, 0.0, 1.0);
    let inner_backdrop = Color::new(1.0, 1.0, 1.0, 1.0);
    let element = Color::new(0.8, 0.4, 0.2, 1.0);

    let mut builder = SceneBuilder::new();
    builder
        .rect(full_rect(), Affine::IDENTITY, page, None, None)
        .unwrap();
    builder
        .group(group(true, 1.0), |outer| {
            outer.rect(full_rect(), Affine::IDENTITY, inner_backdrop, None, None)?;
            outer.group(group(false, 1.0), |inner| {
                blended_rect(inner, element, BlendMode::Multiply)
            })
        })
        .unwrap();
    let got = pixel(
        &render(&mut device, &builder.finish(), W, H),
        W,
        W / 2,
        H / 2,
    );

    // Multiply against a white backdrop is the element itself (§11.3.5: b × s with
    // b = 1), opaque.
    let want = [
        (0.8 * 255.0_f32).round() as u8,
        (0.4 * 255.0_f32).round() as u8,
        (0.2 * 255.0_f32).round() as u8,
        255,
    ];
    for ch in 0..4 {
        let diff = (i32::from(got[ch]) - i32::from(want[ch])).abs();
        assert!(
            diff <= 3,
            "channel {ch}: device {} vs the inner group's own backdrop {} (diff {diff}); \
             a seed taken from the page would read black here",
            got[ch],
            want[ch]
        );
    }
}

// ------------------------------------------------------------------- the refusals

fn non_isolated_error(spec: GroupSpec) -> SceneError {
    let mut builder = SceneBuilder::new();
    builder
        .group(spec, |body| {
            body.rect(
                full_rect(),
                Affine::IDENTITY,
                Color::new(0.0, 0.0, 0.0, 1.0),
                None,
                None,
            )
        })
        .expect_err("this group cannot be drawn and must not be accepted")
}

/// Each of the three conditions, refused by name before the body is built.
#[test]
fn the_three_conditions_are_refused_not_approximated() {
    assert_eq!(
        non_isolated_error(GroupSpec {
            blend: BlendMode::Multiply,
            ..group(false, 1.0)
        }),
        SceneError::NonIsolatedGroupUnsupported {
            reason: NonIsolatedReason::GroupBlendNotNormal
        }
    );
    assert_eq!(
        non_isolated_error(GroupSpec {
            knockout: true,
            ..group(false, 1.0)
        }),
        SceneError::NonIsolatedGroupUnsupported {
            reason: NonIsolatedReason::KnockoutGroup
        }
    );

    // Nested inside a knockout group, whose elements composite with *its* initial
    // backdrop rather than with the accumulated content a seed would copy.
    let mut builder = SceneBuilder::new();
    let error = builder
        .group(
            GroupSpec {
                knockout: true,
                ..group(true, 1.0)
            },
            |outer| {
                outer.group(group(false, 1.0), |inner| {
                    inner.rect(
                        full_rect(),
                        Affine::IDENTITY,
                        Color::new(0.0, 0.0, 0.0, 1.0),
                        None,
                        None,
                    )
                })
            },
        )
        .expect_err("a non-isolated group inside a knockout group cannot be drawn");
    assert_eq!(
        error,
        SceneError::NonIsolatedGroupUnsupported {
            reason: NonIsolatedReason::InsideKnockoutGroup
        }
    );

    // And the builder is usable afterwards: a refusal discards the group whole.
    builder
        .rect(
            full_rect(),
            Affine::IDENTITY,
            Color::new(0.0, 0.0, 0.0, 1.0),
            None,
            None,
        )
        .expect("the builder survives a refusal");
    assert_eq!(builder.finish().cost().commands, 1);
}

/// §11.6.5 renders a soft mask's group on its own, so a knockout group *outside* the
/// mask is not above the mask's content: a non-isolated group inside the mask body is
/// accepted, and the knockout stack starts fresh at the boundary.
#[test]
fn a_mask_body_starts_a_fresh_knockout_stack() {
    let mut builder = SceneBuilder::new();
    builder
        .group(
            GroupSpec {
                knockout: true,
                ..group(true, 1.0)
            },
            |outer| {
                let mask = outer.mask(MaskKind::Alpha, None, |body| {
                    body.group(group(false, 1.0), |inner| {
                        inner.rect(
                            full_rect(),
                            Affine::IDENTITY,
                            Color::new(1.0, 1.0, 1.0, 1.0),
                            None,
                            None,
                        )
                    })
                })?;
                outer.rect(
                    full_rect(),
                    Affine::IDENTITY,
                    Color::new(0.0, 0.0, 0.0, 1.0),
                    None,
                    Some(mask),
                )
            },
        )
        .expect("a mask group's own content is not inside the enclosing knockout group");
}
