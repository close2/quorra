//! The three values this module's tests are written in terms of.
//!
//! Not a helper drawer: a unit rectangle, opaque black, and a group whose every entry
//! is its default are what "an input that is fine" means for the scene, and defining
//! them once is what keeps a test that varies *one* of them honest about which one it
//! varied.

use crate::blend::{BlendMode, Compose};
use crate::geom::{Point, Rect};
use crate::paint::Color;
use crate::scene::GroupSpec;

pub(super) fn unit_rect() -> Rect {
    Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0))
}

pub(super) fn black() -> Color {
    Color::new(0.0, 0.0, 0.0, 1.0)
}

pub(super) fn plain_group() -> GroupSpec {
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
