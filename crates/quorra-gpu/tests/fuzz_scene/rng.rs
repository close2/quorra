//! The value spread: what numbers a fuzzed scene is built from.
//!
//! One responsibility — the deterministic stream, and the proportions in which it
//! yields ordinary, degenerate and outright hostile values. Every method here states
//! its proportion beside itself, because the proportion is the design: a spread that is
//! hostile in every draw refuses at the first call and fuzzes nothing behind it, and a
//! spread that is never hostile fuzzes no refusal at all.
//!
//! Nothing here reads a clock, a device, or `std::random`. A seed replays exactly.

// Test-file lint policy as in m1.rs; the arithmetic here is the fuzzer's own bounded
// index/seed math, not boundary code.
#![allow(
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects,
    clippy::manual_is_multiple_of
)]

use quorra_scene::{Affine, BlendMode, Color, Compose, FillRule, ImageFilter, Point, Rect};

/// Deterministic xorshift64*: reproducible across runs and platforms, so a failure
/// here is a failure everyone can replay from the printed seed.
#[derive(Debug)]
pub(crate) struct Rng(u64);

impl Rng {
    /// A stream from a seed.
    pub(crate) fn seeded(seed: u64) -> Self {
        Self(seed)
    }

    pub(crate) fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub(crate) fn one_in(&mut self, n: u64) -> bool {
        self.next() % n == 0
    }

    /// A hostile spread on purpose: ordinary values, huge values, negatives, NaN and
    /// infinities. Everything §4.7 says to refuse loudly is in here.
    pub(crate) fn f32(&mut self) -> f32 {
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

    /// A number in `0..=1`, always valid — the range colours, alphas and ramp offsets
    /// are defined on.
    pub(crate) fn unit(&mut self) -> f32 {
        (self.next() % 101) as f32 / 100.0
    }

    /// An alpha: in range seven times in eight, and drawn from the hostile spread
    /// otherwise — the `SceneError::InvalidGroupAlpha` path on a group and the
    /// `SceneError::InvalidImageAlpha` one on an image, which are the same range
    /// (ISO 32000-2 §11.3.7.2) refused under the name of what carried it.
    pub(crate) fn alpha(&mut self) -> f32 {
        if self.one_in(8) {
            self.f32()
        } else {
            self.unit()
        }
    }

    pub(crate) fn color(&mut self) -> Color {
        if self.one_in(8) {
            Color::new(self.f32(), self.unit(), self.unit(), self.f32())
        } else {
            Color::new(self.unit(), self.unit(), self.unit(), self.unit())
        }
    }

    /// A coordinate the boundary accepts, biased towards a handful of repeated values
    /// so that degenerate geometry — a zero-length shading axis, two coincident
    /// circles, a zero scale — arises rather than needing two independent floats to
    /// collide.
    pub(crate) fn tame(&mut self) -> f32 {
        match self.next() % 6 {
            0 => 0.0,
            1 => 16.0,
            2 => 32.0,
            _ => ((self.next() % 400) as f32 / 4.0) - 25.0,
        }
    }

    /// A coordinate that is usually drawable and occasionally §4.7's problem.
    pub(crate) fn coord(&mut self) -> f32 {
        if self.one_in(8) {
            self.f32()
        } else {
            self.tame()
        }
    }

    pub(crate) fn point(&mut self) -> Point {
        Point::new(self.coord(), self.coord())
    }

    /// A transform: drawable three times in four, because a scene refused at its first
    /// call never reaches the lanes this fuzzer is aimed at.
    pub(crate) fn affine(&mut self) -> Affine {
        match self.next() % 8 {
            0 => Affine::IDENTITY,
            1 => Affine::translate(self.tame(), self.tame()),
            2 => Affine::scale(self.tame(), self.tame()),
            3 => Affine {
                a: self.tame(),
                b: self.tame(),
                c: self.tame(),
                d: self.tame(),
                e: self.tame(),
                f: self.tame(),
            },
            // Singular: rank one, determinant exactly zero. Every lane that maps a
            // placement back to its source — the image lane above all — inverts this,
            // and an inverse that is not refused is a page of NaN.
            4 => Affine {
                a: 1.0,
                b: 2.0,
                c: 2.0,
                d: 4.0,
                e: self.tame(),
                f: self.tame(),
            },
            5 => Affine::translate(self.f32(), self.f32()),
            6 => Affine::scale(self.f32(), self.f32()),
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

    /// A rectangle: ordered and finite three times in four — a page is mostly
    /// rectangles, and the rect lane is only fuzzed by the ones that get through — and
    /// four values off the hostile spread otherwise, which is where the non-finite,
    /// unordered and too-large refusals live.
    pub(crate) fn rect(&mut self) -> Rect {
        if self.one_in(4) {
            return Rect::new(
                Point::new(self.f32(), self.f32()),
                Point::new(self.f32(), self.f32()),
            );
        }
        self.tame_rect()
    }

    /// A rectangle the boundary always accepts: finite, ordered, and inside the
    /// coordinate bound.
    pub(crate) fn tame_rect(&mut self) -> Rect {
        let min = Point::new(self.tame(), self.tame());
        Rect::new(
            min,
            Point::new(min.x + self.unit() * 40.0, min.y + self.unit() * 40.0),
        )
    }

    /// A mesh anchor, including both ends of the type: a mesh is placed at absolute
    /// device pixels, so `i32::MIN` is the arithmetic the placement has to survive.
    pub(crate) fn i32(&mut self) -> i32 {
        match self.next() % 6 {
            0 => i32::MIN,
            1 => i32::MAX,
            2 => 0,
            3 => -i32::try_from(self.next() % 100_000).unwrap(),
            _ => i32::try_from(self.next() % 64).unwrap(),
        }
    }

    pub(crate) fn blend(&mut self) -> BlendMode {
        // Every separable and non-separable mode of §11.3.5, so the exotic ones —
        // which compile lazily and composite through a different path — are reached.
        const MODES: [BlendMode; 16] = [
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
        // Normal half the time: it is what an ordinary page is, and it is the mode
        // several of the group refusals are conditioned on.
        if self.one_in(2) {
            BlendMode::Normal
        } else {
            MODES[usize::try_from(self.next()).unwrap_or(0) % MODES.len()]
        }
    }

    /// A compositing operator: §11.4.6's three, and the ordinary one the rest of the
    /// time. Each is refused in some positions and accepted in others (ADR 0033).
    pub(crate) fn compose(&mut self) -> Compose {
        match self.next() % 6 {
            0 => Compose::Src,
            1 => Compose::DestOut,
            2 => Compose::Plus,
            _ => Compose::SrcOver,
        }
    }

    pub(crate) fn filter(&mut self) -> ImageFilter {
        if self.one_in(2) {
            ImageFilter::Nearest
        } else {
            ImageFilter::Linear
        }
    }

    pub(crate) fn fill_rule(&mut self) -> FillRule {
        if self.one_in(2) {
            FillRule::NonZero
        } else {
            FillRule::EvenOdd
        }
    }
}
