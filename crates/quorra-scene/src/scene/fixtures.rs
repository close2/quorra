//! The four values this module's tests are written in terms of.
//!
//! Not a helper drawer: a unit rectangle, opaque black, a group whose every entry is its
//! default, and the smallest well-formed function paint are what "an input that is fine"
//! means for the scene, and defining them once is what keeps a test that varies *one* of
//! them honest about which one it varied.

use crate::blend::{BlendMode, Compose};
use crate::function::FnRange;
use crate::geom::{Affine, Point, Rect};
use crate::ids::FunctionId;
use crate::paint::{Color, Paint};
use crate::scene::GroupSpec;

pub(super) fn unit_rect() -> Rect {
    Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0))
}

pub(super) fn black() -> Color {
    Color::new(0.0, 0.0, 0.0, 1.0)
}

/// A §8.7.4.5.2 type 1 shading over the unit square, in `DeviceGray`, with no background
/// — the shape both of the caller's witnesses declare (`/Domain [0 1 0 1]`), so that a
/// test varying one entry is varying exactly one.
pub(super) fn function_paint(program: FunctionId) -> Paint {
    Paint::Function {
        program,
        domain: unit_rect(),
        matrix: Affine::IDENTITY,
        range: FnRange::Gray([0.0, 1.0]),
        background: None,
    }
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
