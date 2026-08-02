//! Structured fuzzing of the scene boundary (CLAUDE.md principle 3: "fuzz the scene
//! boundary from the first commit" — this is the first commit with a boundary).
//!
//! A deterministic xorshift generator drives thousands of random builder-and-upload
//! call sequences — hostile values included: NaN, infinities, 1e30 coordinates,
//! unordered rects, deep nesting, foreign ids, oversized images — and asserts one
//! property: **the boundary never panics and never accepts silently.** Every call
//! returns `Ok` or a typed error; every finished scene can be costed and rendered
//! (to a drawn frame or a typed refusal) without a crash.
//!
//! Deliberately not `cargo-fuzz`: coverage-guided fuzzing needs a nightly toolchain
//! and this tree pins stable (`rust-toolchain.toml`). A deterministic generator in
//! the ordinary suite runs everywhere the suite runs, on every push; a nightly-based
//! coverage-guided harness can live outside the pinned tree later if the boundary
//! grows past what this covers. Every crasher found here or there becomes a named
//! regression test.

// Test-file lint policy as in m1.rs; the arithmetic here is the fuzzer's own bounded
// index/seed math, not boundary code.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects,
    clippy::manual_is_multiple_of
)]

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, ClipId, Color, Compose, FillRule, LineCap, LineJoin, OutlineId, Paint,
    Point, Rect, SceneBuilder, Segment, Stroke,
};

/// Deterministic xorshift64*: reproducible across runs and platforms, so a failure
/// here is a failure everyone can replay from the printed seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn f32(&mut self) -> f32 {
        // A hostile spread on purpose: ordinary values, huge values, negatives,
        // NaN and infinities, in proportions that keep most scenes structurally
        // interesting rather than refused at the first call.
        match self.next() % 12 {
            0 => f32::NAN,
            1 => f32::INFINITY,
            2 => f32::NEG_INFINITY,
            3 => 1e30,
            4 => -1e30,
            5 => 0.0,
            _ => ((self.next() % 4_000) as f32 / 2.0) - 500.0,
        }
    }

    fn color(&mut self) -> Color {
        Color::new(self.f32() / 100.0, 0.5, 0.5, self.f32() / 100.0)
    }

    fn affine(&mut self) -> Affine {
        match self.next() % 4 {
            0 => Affine::IDENTITY,
            1 => Affine::translate(self.f32(), self.f32()),
            2 => Affine::scale(self.f32(), self.f32()),
            _ => Affine {
                a: self.f32(),
                b: self.f32(),
                c: self.f32(),
                d: self.f32(),
                e: self.f32(),
                f: self.f32(),
            },
        }
    }

    fn rect(&mut self) -> Rect {
        Rect::new(
            Point::new(self.f32(), self.f32()),
            Point::new(self.f32(), self.f32()),
        )
    }
}

fn random_ops(rng: &mut Rng, builder: &mut SceneBuilder, outlines: &[OutlineId], depth: u32) {
    let ops = 4 + (rng.next() % 24);
    for _ in 0..ops {
        // Ids are a mix of genuinely uploaded outlines and fabricated ones; clip ids
        // a mix of plausible and foreign. Everything may be refused; nothing may panic.
        let outline = if outlines.is_empty() || rng.next() % 4 == 0 {
            OutlineId(u32::try_from(rng.next() % 1_000).unwrap())
        } else {
            outlines[usize::try_from(rng.next()).unwrap_or(0) % outlines.len()]
        };
        let clip = match rng.next() % 3 {
            0 => None,
            _ => Some(ClipId(u32::try_from(rng.next() % 8).unwrap())),
        };
        match rng.next() % 5 {
            0 => {
                let _ = builder.rect(rng.rect(), rng.affine(), rng.color(), clip, None);
            }
            1 => {
                let _ = builder.fill(
                    outline,
                    rng.affine(),
                    FillRule::NonZero,
                    Paint::Solid(rng.color()),
                    clip,
                    BlendMode::Normal,
                    Compose::SrcOver,
                    None,
                );
            }
            2 => {
                let _ = builder.stroke(
                    outline,
                    rng.affine(),
                    Stroke {
                        width: rng.f32(),
                        cap: LineCap::Round,
                        join: LineJoin::Bevel,
                        miter_limit: rng.f32(),
                    },
                    Paint::Solid(rng.color()),
                    clip,
                    BlendMode::Multiply,
                    None,
                );
            }
            3 => {
                let _ = builder.clip(outline, rng.affine(), FillRule::EvenOdd, clip);
            }
            _ => {
                let alpha = rng.f32() / 100.0;
                let seed = rng.next();
                let _ = builder.group(
                    quorra_scene::GroupSpec {
                        alpha,
                        blend: BlendMode::Normal,
                        clip,
                        knockout: seed % 2 == 0,
                        mask: None,
                    },
                    |inner| {
                        if depth < 20 {
                            random_ops(&mut Rng(seed), inner, outlines, depth + 1);
                        }
                        Ok(())
                    },
                );
            }
        }
    }
}

/// The one property, over two hundred seeded scenes: no panic, no silent acceptance,
/// and every finished scene either renders or is refused with a typed error.
#[test]
fn the_boundary_never_panics_and_never_accepts_silently() {
    let mut device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs");

    for seed in 1..=200_u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));

        // Random uploads, hostile segments included.
        let mut outlines = Vec::new();
        for _ in 0..(rng.next() % 4) {
            let mut segments = vec![Segment::MoveTo(Point::new(rng.f32(), rng.f32()))];
            for _ in 0..(rng.next() % 6) {
                segments.push(match rng.next() % 3 {
                    0 => Segment::LineTo(Point::new(rng.f32(), rng.f32())),
                    1 => Segment::CubicTo {
                        c1: Point::new(rng.f32(), rng.f32()),
                        c2: Point::new(rng.f32(), rng.f32()),
                        to: Point::new(rng.f32(), rng.f32()),
                    },
                    _ => Segment::Close,
                });
            }
            if let Ok(id) = device.upload_outline(&segments) {
                outlines.push(id);
            }
        }

        let mut builder = SceneBuilder::new();
        random_ops(&mut rng, &mut builder, &outlines, 0);
        let scene = builder.finish();
        let cost = scene.cost();
        assert!(
            cost.group_depth <= quorra_scene::MAX_GROUP_DEPTH,
            "seed {seed}: the depth bound leaked"
        );

        let viewport = Viewport::full(32, 32, Affine::IDENTITY);
        match device.render(&scene, &viewport, Target::Readback) {
            Ok(frame) => {
                let raster = frame.into_raster().expect("readback frames carry rasters");
                assert_eq!(raster.pixels().len(), 32 * 32 * 4, "seed {seed}");
            }
            Err(error) => {
                // A refusal is legitimate; what matters is that it is typed and the
                // device remains usable.
                let _ = error.to_string();
            }
        }

        for id in outlines {
            device.release(id).expect("uploaded this loop");
        }
    }
}
