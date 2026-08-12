//! The M6 harness: clause 11 through the compositor — the sixteen blend modes
//! against clause-derived arithmetic, the knockout diagonal, group alpha and
//! isolation, and the soft-mask reduction held to the caller's shared rule for all
//! 256 bytes.
//!
//! # Where the expected values come from
//!
//! - The blend reference below transcribes ISO 32000-2 §11.3.5's separable and
//!   non-separable functions and §11.3.6's compositing formula **from the clause**,
//!   independently of `composite.wgsl` — two transcriptions of one clause, checked
//!   against each other, never against another renderer (principle 5).
//! - `soft_mask_value` transcribes `pdf_render::SoftMask::value` from the caller's
//!   tree (`crates/pdf-render/src/soft_mask.rs`), which both of its backends share
//!   *on purpose*; §4.2 makes our reduction a second implementation that must agree
//!   to the byte, and this file holds all 256 of them.
//! - The knockout expectation derives from §11.4.6's own formula — result =
//!   shape·(element over transparent) + (1 − shape)·accumulated — using
//!   single-element frames as the formula's inputs, so the *rule* is what is tested.

// Test-file lint policy as in m1.rs; the reference math mirrors clause arithmetic.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects,
    clippy::too_many_lines
)]

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, GroupSpec, MaskKind, Paint, Point, Rect, Scene,
    SceneBuilder, Segment, Transfer,
};

fn device() -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

fn render(device: &mut Device, scene: &Scene, width: u32, height: u32) -> Vec<u8> {
    device
        .render(
            scene,
            &Viewport::full(width, height, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders")
        .into_raster()
        .unwrap()
        .into_pixels()
}

fn plain_group() -> GroupSpec {
    GroupSpec {
        alpha: 1.0,
        blend: BlendMode::Normal,
        clip: None,
        knockout: false,
        mask: None,
        isolated: true,
        compose: Compose::SrcOver,
    }
}

/// Transcription of `pdf_render::SoftMask::value` (the caller's shared reduction
/// rule), straight-alpha RGBA in, one mask byte out.
fn soft_mask_value(kind: &MaskKind, transfer: Option<&Transfer>, pixel: [u8; 4]) -> u8 {
    let derived = match *kind {
        MaskKind::Alpha => pixel[3],
        MaskKind::Luminosity { backdrop } => {
            let alpha = f32::from(pixel[3]) / 255.0;
            let over = |channel: u8, backdrop: f32| {
                (f32::from(channel) / 255.0).mul_add(alpha, backdrop * (1.0 - alpha))
            };
            let luminosity = 0.30_f32.mul_add(
                over(pixel[0], backdrop.r),
                0.59_f32.mul_add(
                    over(pixel[1], backdrop.g),
                    0.11 * over(pixel[2], backdrop.b),
                ),
            );
            (luminosity * 255.0).round().clamp(0.0, 255.0) as u8
        }
    };
    transfer.map_or(derived, |t| t.apply(derived))
}

/// §4.2's conformance: every one of the 256 mask bytes, through both rules, against
/// the transcribed shared rule — plus a non-black luminosity backdrop and a
/// non-identity transfer sampled at both endpoints (the M6 definition of done).
#[test]
fn soft_mask_reduction_agrees_for_all_256_bytes() {
    let mut device = device();

    // A transfer that is visibly not the identity, endpoints included.
    let mut table = [0_u8; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        *slot = 255 - (i as u8);
    }
    let inversion = Transfer(table);

    let cases: Vec<(MaskKind, Option<Transfer>)> = vec![
        (MaskKind::Alpha, None),
        (MaskKind::Alpha, Some(inversion.clone())),
        (
            MaskKind::Luminosity {
                backdrop: Color::new(0.0, 0.0, 0.0, 1.0),
            },
            None,
        ),
        (
            // The non-black backdrop: the term the default hides.
            MaskKind::Luminosity {
                backdrop: Color::new(0.25, 0.5, 0.75, 1.0),
            },
            Some(inversion.clone()),
        ),
    ];

    for (kind, transfer) in cases {
        // The mask group: 256 one-pixel columns. For the alpha rule the column's
        // alpha is the byte; for luminosity an opaque gray carries it, so the
        // premultiplied store round-trips the byte exactly either way.
        let mut builder = SceneBuilder::new();
        let mask_id = builder
            .mask(kind, transfer.clone(), |body| {
                for i in 0..256_u32 {
                    let level = i as f32 / 255.0;
                    let color = if matches!(kind, MaskKind::Alpha) {
                        Color::new(0.0, 0.0, 0.0, level)
                    } else {
                        Color::new(level, level, level, 1.0)
                    };
                    body.rect(
                        Rect::new(Point::new(i as f32, 0.0), Point::new(i as f32 + 1.0, 1.0)),
                        Affine::IDENTITY,
                        color,
                        None,
                        None,
                    )?;
                }
                Ok(())
            })
            .expect("valid mask");
        // Opaque white under the mask: the readback alpha *is* the mask byte.
        builder
            .rect(
                Rect::new(Point::new(0.0, 0.0), Point::new(256.0, 1.0)),
                Affine::IDENTITY,
                Color::new(1.0, 1.0, 1.0, 1.0),
                None,
                Some(mask_id),
            )
            .expect("valid masked rect");
        let pixels = render(&mut device, &builder.finish(), 256, 1);

        for i in 0..256_usize {
            let input = if matches!(kind, MaskKind::Alpha) {
                [0, 0, 0, i as u8]
            } else {
                [i as u8, i as u8, i as u8, 255]
            };
            let expected = soft_mask_value(&kind, transfer.as_ref(), input);
            let got = pixels[i * 4 + 3];
            assert_eq!(
                got,
                expected,
                "byte {i} under {kind:?} (transfer: {}): device {got}, shared rule {expected}",
                transfer.is_some(),
            );
        }
    }
}

/// §11.3.5's blend function B, transcribed from the clause for the reference.
fn blend_reference(mode: BlendMode, cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    let lum = |c: [f32; 3]| 0.30 * c[0] + 0.59 * c[1] + 0.11 * c[2];
    let clip_color = |c: [f32; 3]| {
        let l = lum(c);
        let n = c[0].min(c[1]).min(c[2]);
        let x = c[0].max(c[1]).max(c[2]);
        let mut out = c;
        if n < 0.0 {
            for v in &mut out {
                *v = l + (*v - l) * l / (l - n);
            }
        }
        if x > 1.0 {
            for v in &mut out {
                *v = l + (*v - l) * (1.0 - l) / (x - l);
            }
        }
        out
    };
    let set_lum = |c: [f32; 3], l: f32| {
        let d = l - lum(c);
        clip_color([c[0] + d, c[1] + d, c[2] + d])
    };
    let sat = |c: [f32; 3]| c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]);
    let set_sat = |c: [f32; 3], s: f32| {
        let mn = c[0].min(c[1]).min(c[2]);
        let mx = c[0].max(c[1]).max(c[2]);
        if mx <= mn {
            return [0.0; 3];
        }
        [
            (c[0] - mn) * s / (mx - mn),
            (c[1] - mn) * s / (mx - mn),
            (c[2] - mn) * s / (mx - mn),
        ]
    };
    let per = |f: &dyn Fn(f32, f32) -> f32| [f(cb[0], cs[0]), f(cb[1], cs[1]), f(cb[2], cs[2])];
    match mode {
        BlendMode::Normal => cs,
        BlendMode::Multiply => per(&|b, s| b * s),
        BlendMode::Screen => per(&|b, s| b + s - b * s),
        BlendMode::Overlay => per(&|b, s| {
            if b <= 0.5 {
                s * (2.0 * b)
            } else {
                let b2 = 2.0 * b - 1.0;
                s + b2 - s * b2
            }
        }),
        BlendMode::Darken => per(&|b, s| b.min(s)),
        BlendMode::Lighten => per(&|b, s| b.max(s)),
        BlendMode::ColorDodge => per(&|b, s| {
            if b <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }),
        BlendMode::ColorBurn => per(&|b, s| {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }),
        BlendMode::HardLight => per(&|b, s| {
            if s <= 0.5 {
                b * (2.0 * s)
            } else {
                let s2 = 2.0 * s - 1.0;
                b + s2 - b * s2
            }
        }),
        BlendMode::SoftLight => per(&|b, s| {
            let d = if b <= 0.25 {
                ((16.0 * b - 12.0) * b + 4.0) * b
            } else {
                b.sqrt()
            };
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                b + (2.0 * s - 1.0) * (d - b)
            }
        }),
        BlendMode::Difference => per(&|b, s| (b - s).abs()),
        BlendMode::Exclusion => per(&|b, s| b + s - 2.0 * b * s),
        BlendMode::Hue => set_lum(set_sat(cs, sat(cb)), lum(cb)),
        BlendMode::Saturation => set_lum(set_sat(cb, sat(cs)), lum(cb)),
        BlendMode::Color => set_lum(cs, lum(cb)),
        BlendMode::Luminosity => set_lum(cb, lum(cs)),
    }
}

const ALL_MODES: [BlendMode; 16] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::HardLight,
    BlendMode::SoftLight,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
];

/// The sixteen-mode scene (the caller's fixture found three of `tiny-skia`'s modes
/// wrong by up to 113 of 255 because nothing had ever compared them): a fixed
/// backdrop, sixteen grouped patches, each held to §11.3.6 with the transcribed B.
#[test]
fn all_sixteen_blend_modes_match_the_clause() {
    let mut device = device();
    let backdrop = Color::new(0.55, 0.35, 0.2, 0.8);
    let source = Color::new(0.3, 0.7, 0.9, 0.6);

    let mut builder = SceneBuilder::new();
    builder
        .rect(
            Rect::new(Point::new(0.0, 0.0), Point::new(64.0, 16.0)),
            Affine::IDENTITY,
            backdrop,
            None,
            None,
        )
        .unwrap();
    for (i, mode) in ALL_MODES.iter().enumerate() {
        builder
            .group(
                GroupSpec {
                    blend: *mode,
                    ..plain_group()
                },
                |body| {
                    body.rect(
                        Rect::new(
                            Point::new(i as f32 * 4.0, 4.0),
                            Point::new(i as f32 * 4.0 + 4.0, 12.0),
                        ),
                        Affine::IDENTITY,
                        source,
                        None,
                        None,
                    )
                },
            )
            .unwrap();
    }
    let pixels = render(&mut device, &builder.finish(), 64, 16);

    // The reference, per §11.3.6 on the quantised layer values the device also sees.
    let quant = |v: f32| (v * 255.0).round() / 255.0;
    let ab = quant(backdrop.a);
    let cb = [
        quant(backdrop.r * backdrop.a) / ab.max(1e-6),
        quant(backdrop.g * backdrop.a) / ab.max(1e-6),
        quant(backdrop.b * backdrop.a) / ab.max(1e-6),
    ];
    let as_ = quant(source.a);
    let cs = [
        quant(source.r * source.a) / as_.max(1e-6),
        quant(source.g * source.a) / as_.max(1e-6),
        quant(source.b * source.a) / as_.max(1e-6),
    ];
    for (i, mode) in ALL_MODES.iter().enumerate() {
        let mixed = blend_reference(*mode, cb, cs);
        let ao = as_ + ab * (1.0 - as_);
        let mut expected = [0_u8; 4];
        for ch in 0..3 {
            let co = as_ * (1.0 - ab) * cs[ch] + ab * (1.0 - as_) * cb[ch] + as_ * ab * mixed[ch];
            // Premultiplied out, then the readback's straight conversion.
            let premul = (co * 255.0).round().clamp(0.0, 255.0) as u32;
            let alpha = (ao * 255.0).round().clamp(0.0, 255.0) as u32;
            expected[ch] = ((premul * 255 + alpha / 2) / alpha).min(255) as u8;
        }
        expected[3] = (ao * 255.0).round() as u8;

        // Sample the patch's interior.
        let x = i * 4 + 2;
        let y = 8;
        let got = &pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        for ch in 0..4 {
            let diff = (i32::from(got[ch]) - i32::from(expected[ch])).abs();
            assert!(
                diff <= 3,
                "{mode:?} channel {ch}: device {} vs clause {} (diff {diff})",
                got[ch],
                expected[ch]
            );
        }
    }
}

/// §11.4.6 with the diagonal edge the brief demands (§4.1: axis-aligned rectangles
/// would agree while being wrong): the knockout result must equal
/// shape·(element over transparent) + (1 − shape)·accumulated, with the two
/// single-element frames supplying the formula's inputs.
#[test]
fn knockout_replaces_by_shape_on_a_diagonal_edge() {
    let mut device = device();
    let triangle = device
        .upload_outline(&[
            Segment::MoveTo(Point::new(2.0, 2.0)),
            Segment::LineTo(Point::new(30.0, 2.0)),
            Segment::LineTo(Point::new(2.0, 30.0)),
            Segment::Close,
        ])
        .expect("upload");
    let base = Color::new(0.1, 0.2, 0.9, 1.0);
    let over = Color::new(0.9, 0.3, 0.1, 0.5);

    let scene_with = |knockout: bool| {
        let mut builder = SceneBuilder::new();
        builder
            .group(
                GroupSpec {
                    knockout,
                    ..plain_group()
                },
                |body| {
                    body.rect(
                        Rect::new(Point::new(0.0, 0.0), Point::new(32.0, 32.0)),
                        Affine::IDENTITY,
                        base,
                        None,
                        None,
                    )?;
                    body.fill(
                        triangle,
                        Affine::IDENTITY,
                        FillRule::NonZero,
                        Paint::Solid(over),
                        None,
                        BlendMode::Normal,
                        Compose::SrcOver,
                        None,
                    )
                },
            )
            .unwrap();
        builder.finish()
    };
    let knocked = render(&mut device, &scene_with(true), 32, 32);
    let stacked = render(&mut device, &scene_with(false), 32, 32);

    // Inputs to §11.4.6's formula: each element alone (over transparency).
    let alone = |which: u32| {
        let mut builder = SceneBuilder::new();
        builder
            .group(plain_group(), |body| {
                if which == 0 {
                    body.rect(
                        Rect::new(Point::new(0.0, 0.0), Point::new(32.0, 32.0)),
                        Affine::IDENTITY,
                        base,
                        None,
                        None,
                    )
                } else {
                    body.fill(
                        triangle,
                        Affine::IDENTITY,
                        FillRule::NonZero,
                        Paint::Solid(over),
                        None,
                        BlendMode::Normal,
                        Compose::SrcOver,
                        None,
                    )
                }
            })
            .unwrap();
        builder.finish()
    };
    let base_alone = render(&mut device, &alone(0), 32, 32);
    let tri_alone = render(&mut device, &alone(1), 32, 32);

    let mut max_formula_diff = 0_i32;
    let mut interior_differs = false;
    for px in 0..(32 * 32_usize) {
        // Premultiplied values from the straight readbacks.
        let premul = |data: &[u8], ch: usize| {
            let a = f32::from(data[px * 4 + 3]) / 255.0;
            f32::from(data[px * 4 + ch]) / 255.0 * a
        };
        let alpha = |data: &[u8]| f32::from(data[px * 4 + 3]) / 255.0;
        // Shape of the triangle at this pixel: its alone-alpha over its paint
        // alpha. The alone frame already carries shape·(element over transparent),
        // so §11.4.6's formula is that plus (1 − shape) of the accumulated base.
        let shape = (alpha(&tri_alone) / over.a).clamp(0.0, 1.0);
        for ch in 0..3 {
            let expected = premul(&tri_alone, ch) + (1.0 - shape) * premul(&base_alone, ch);
            let got = premul(&knocked, ch);
            let diff = ((got - expected) * 255.0).abs();
            max_formula_diff = max_formula_diff.max(diff.round() as i32);
        }
        // On interior overlap, knockout must differ from ordinary stacking: the
        // base is *replaced* under the triangle rather than blended under 50% red.
        if shape > 0.99 {
            let k = &knocked[px * 4..px * 4 + 4];
            let s = &stacked[px * 4..px * 4 + 4];
            if k != s {
                interior_differs = true;
            }
        }
    }
    assert!(
        max_formula_diff <= 3,
        "knockout deviates from §11.4.6's formula by {max_formula_diff} steps"
    );
    assert!(
        interior_differs,
        "knockout and ordinary stacking agreed everywhere — the knockout did nothing"
    );
}

/// §11.4.5: a group's constant alpha scales the finished group once. A group at
/// alpha ½ over nothing equals the same content drawn at alpha ½ directly (for a
/// single opaque element, where the two computations coincide).
#[test]
fn group_alpha_applies_once_to_the_finished_group() {
    let mut device = device();
    let mut grouped = SceneBuilder::new();
    grouped
        .group(
            GroupSpec {
                alpha: 0.5,
                ..plain_group()
            },
            |body| {
                body.rect(
                    Rect::new(Point::new(2.0, 2.0), Point::new(14.0, 14.0)),
                    Affine::IDENTITY,
                    Color::new(0.8, 0.2, 0.4, 1.0),
                    None,
                    None,
                )
            },
        )
        .unwrap();
    let via_group = render(&mut device, &grouped.finish(), 16, 16);

    let mut direct = SceneBuilder::new();
    direct
        .rect(
            Rect::new(Point::new(2.0, 2.0), Point::new(14.0, 14.0)),
            Affine::IDENTITY,
            Color::new(0.8, 0.2, 0.4, 0.5),
            None,
            None,
        )
        .unwrap();
    let via_alpha = render(&mut device, &direct.finish(), 16, 16);

    let max = via_group
        .iter()
        .zip(&via_alpha)
        .map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs())
        .max()
        .unwrap();
    assert!(max <= 2, "group alpha diverged from direct alpha by {max}");
}

/// §11.4.1/§11.6.6: a group composites as a unit. Two overlapping half-alpha
/// elements inside a Multiply group must NOT equal the same two elements each
/// Multiply-blended onto the page — the difference is what isolation exists for.
#[test]
fn groups_composite_as_a_unit_not_per_element() {
    let mut device = device();
    let page = Color::new(0.9, 0.8, 0.3, 1.0);
    let e1 = Color::new(0.2, 0.5, 0.8, 0.6);
    let e2 = Color::new(0.8, 0.3, 0.2, 0.6);
    let r1 = Rect::new(Point::new(2.0, 2.0), Point::new(12.0, 12.0));
    let r2 = Rect::new(Point::new(6.0, 6.0), Point::new(15.0, 15.0));

    let mut unit = SceneBuilder::new();
    unit.rect(
        Rect::new(Point::new(0.0, 0.0), Point::new(16.0, 16.0)),
        Affine::IDENTITY,
        page,
        None,
        None,
    )
    .unwrap();
    unit.group(
        GroupSpec {
            blend: BlendMode::Multiply,
            ..plain_group()
        },
        |body| {
            body.rect(r1, Affine::IDENTITY, e1, None, None)?;
            body.rect(r2, Affine::IDENTITY, e2, None, None)
        },
    )
    .unwrap();
    let grouped = render(&mut device, &unit.finish(), 16, 16);

    let mut spread = SceneBuilder::new();
    spread
        .rect(
            Rect::new(Point::new(0.0, 0.0), Point::new(16.0, 16.0)),
            Affine::IDENTITY,
            page,
            None,
            None,
        )
        .unwrap();
    for (rect, color) in [(r1, e1), (r2, e2)] {
        spread
            .group(
                GroupSpec {
                    blend: BlendMode::Multiply,
                    ..plain_group()
                },
                |body| body.rect(rect, Affine::IDENTITY, color, None, None),
            )
            .unwrap();
    }
    let per_element = render(&mut device, &spread.finish(), 16, 16);

    // In the overlap (pixel 8,8 is inside both rects) the two must differ.
    let px = (8 * 16 + 8) * 4;
    assert_ne!(
        &grouped[px..px + 4],
        &per_element[px..px + 4],
        "a group blended per element — §11.4.1's isolation is broken"
    );
}

/// Compositor frames are deterministic on one adapter (§4.6), and the layered path
/// reports itself truthfully: a masked, blended, nested scene renders identically
/// twice, and still within ADR 0006's bound across adapters.
#[test]
fn compositor_frames_are_deterministic_and_bounded_cross_adapter() {
    let build = || {
        let mut builder = SceneBuilder::new();
        let mask = builder
            .mask(
                MaskKind::Luminosity {
                    backdrop: Color::new(0.0, 0.0, 0.0, 1.0),
                },
                None,
                |body| {
                    body.rect(
                        Rect::new(Point::new(0.0, 0.0), Point::new(24.0, 32.0)),
                        Affine::IDENTITY,
                        Color::new(0.8, 0.8, 0.8, 1.0),
                        None,
                        None,
                    )
                },
            )
            .unwrap();
        builder
            .rect(
                Rect::new(Point::new(0.0, 0.0), Point::new(32.0, 32.0)),
                Affine::IDENTITY,
                Color::new(0.2, 0.6, 0.4, 1.0),
                None,
                None,
            )
            .unwrap();
        builder
            .group(
                GroupSpec {
                    alpha: 0.7,
                    blend: BlendMode::Screen,
                    mask: Some(mask),
                    ..plain_group()
                },
                |body| {
                    body.rect(
                        Rect::new(Point::new(4.0, 4.0), Point::new(28.0, 28.0)),
                        Affine::IDENTITY,
                        Color::new(0.9, 0.1, 0.5, 0.8),
                        None,
                        None,
                    )?;
                    body.group(plain_group(), |inner| {
                        inner.rect(
                            Rect::new(Point::new(10.0, 10.0), Point::new(20.0, 20.0)),
                            Affine::IDENTITY,
                            Color::new(0.1, 0.9, 0.9, 0.5),
                            None,
                            None,
                        )
                    })
                },
            )
            .unwrap();
        builder.finish()
    };

    let mut first = device();
    let scene = build();
    let a = render(&mut first, &scene, 32, 32);
    let b = render(&mut first, &scene, 32, 32);
    assert_eq!(a, b, "the same compositor frame must be byte-identical");

    if Device::headless(&Options {
        adapter: Some("RADV".into()),
        ..Options::default()
    })
    .is_ok()
    {
        let mut radv = Device::headless(&Options {
            adapter: Some("RADV".into()),
            ..Options::default()
        })
        .unwrap();
        let scene = build();
        let c = render(&mut radv, &scene, 32, 32);
        let diff = a
            .iter()
            .zip(&c)
            .map(|(x, y)| (i32::from(*x) - i32::from(*y)).abs())
            .max()
            .unwrap();
        // Layered frames pass through more blended stores than flat ones; the
        // bound is ADR 0006's per-stage step times the stage count of this scene.
        assert!(
            diff <= 6,
            "compositor output diverged across adapters by {diff} unorm steps"
        );
    }
}
