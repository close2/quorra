//! The glyph page: 5 933 fills over 107 letterforms, the shape three instruments draw.
//!
//! `examples/floor.rs` measures the per-target floor on it, `examples/zoom.rs` prices
//! culling on it, and `examples/retained.rs` runs ADR 0050's atlas-settling section on
//! it. Until 2026-08-17 each held its own copy, one of them under the comment "**verbatim**"
//! — and the three copies were **not** identical: `retained.rs`'s drew in the archetypes'
//! ink (`0.12, 0.13, 0.16`) rather than this page's (`0.1, 0.1, 0.1`). Reconciled here to
//! the ink two of the three carried, which moves no number any of them reports: a solid
//! fill's colour enters no counter, no atlas key and no clock (ADR 0060 §5).

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, OutlineId, Paint, Point, Scene, SceneBuilder,
    SceneError, Segment,
};

/// The glyph page's ink.
const INK: Color = Color::new(0.1, 0.1, 0.1, 1.0);

/// A page of letterforms placed on a grid: the shape the brief's §0 says a document
/// renderer must be fast at, and the one the glyph atlas exists for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphPage {
    /// The page's name.
    pub name: &'static str,
    /// Distinct letterforms, of five widths and seven heights.
    pub distinct: u32,
    /// Placements over them, on a 14.5 × 15.25 grid eighty columns wide.
    pub placements: u32,
    /// Whether each placement takes a fresh sub-pixel phase.
    ///
    /// `false` is maximal atlas reuse — every placement lands on an integer phase and
    /// shares one rasterisation. `true` is the page the atlas cannot help, which is the
    /// other book-end of `examples/floor.rs`'s sweep.
    pub unique_phases: bool,
    /// The window the page is measured in.
    pub width: u32,
    /// The window the page is measured in.
    pub height: u32,
}

/// The glyph page at integer phases: maximal reuse, 107 outlines answering 5 933 fills.
pub const GLYPH_PAGE: GlyphPage = GlyphPage {
    name: "glyph page",
    distinct: 107,
    placements: 5_933,
    unique_phases: false,
    width: 1191,
    height: 1684,
};

/// The same page with every fill at a fresh sub-pixel phase: the page the atlas cannot
/// help, because each placement is its own key.
pub const GLYPH_PAGE_UNIQUE_PHASES: GlyphPage = GlyphPage {
    name: "glyph page, unique phases",
    unique_phases: true,
    ..GLYPH_PAGE
};

/// The letterform paths the page needs uploaded, in the order their identifiers are
/// expected in.
#[must_use]
pub fn glyph_outlines(page: &GlyphPage) -> Vec<Vec<Segment>> {
    (0..page.distinct)
        .map(|i| {
            let (w, h) = (6.0 + (i % 5) as f32, 8.0 + (i % 7) as f32);
            vec![
                Segment::MoveTo(Point::new(0.3, 0.2)),
                Segment::LineTo(Point::new(w, 0.0)),
                Segment::CubicTo {
                    c1: Point::new(w + 1.0, h * 0.3),
                    c2: Point::new(w + 1.0, h * 0.7),
                    to: Point::new(w * 0.8, h),
                },
                Segment::LineTo(Point::new(0.0, h * 0.9)),
                Segment::Close,
            ]
        })
        .collect()
}

/// The page's scene, from letterform identifiers a device has already accepted.
///
/// # Errors
///
/// Whatever `SceneBuilder` refuses; nothing here constructs a refusable scene.
///
/// # Panics
///
/// If `outlines` is empty — a contract violation at the call site rather than data.
pub fn glyph_scene(page: &GlyphPage, outlines: &[OutlineId]) -> Result<Scene, SceneError> {
    assert!(!outlines.is_empty(), "the glyph page needs its letterforms");
    let mut builder = SceneBuilder::new();
    for i in 0..page.placements {
        // A fresh phase per placement is a fresh atlas key; zero is the same key for
        // every placement in a column, which is what makes the two variants opposite
        // ends of one sweep rather than two pages.
        let phase = if page.unique_phases {
            (i as f32) * 0.061_3 % 1.0
        } else {
            0.0
        };
        builder.fill(
            outlines[(i as usize) % outlines.len()],
            Affine::translate((i % 80) as f32 * 14.5 + phase, (i / 80) as f32 * 15.25),
            FillRule::NonZero,
            Paint::Solid(INK),
            None,
            BlendMode::Normal,
            Compose::SrcOver,
            None,
        )?;
    }
    Ok(builder.finish())
}

/// The viewport transform a viewer zoomed to `magnification` about the page's middle
/// would ask for: the window is the same size, the page is larger, and most of it is
/// outside.
#[must_use]
pub fn zoomed(page: &GlyphPage, magnification: f32) -> Affine {
    // The dense page's middle, in page coordinates.
    let (centre_x, centre_y) = (580.0, 565.0);
    Affine::translate(-centre_x, -centre_y)
        .then(Affine::scale(magnification, magnification))
        .then(Affine::translate(
            page.width as f32 / 2.0,
            page.height as f32 / 2.0,
        ))
}
