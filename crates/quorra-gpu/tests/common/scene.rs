//! The scene pieces more than one test file draws.
//!
//! One responsibility: hand back something to put into a `SceneBuilder`. Nothing here
//! renders — that is [`super::headless`]'s business — and nothing here asserts.

use quorra_scene::{Affine, Color, Paint, Point, Rect, Scene, SceneBuilder, Segment};

/// A rectangle as an outline, the way the caller's clips arrive (its display list has
/// no rectangle type — recognition is our side's job, §6.4).
///
/// **Which lane this takes, since ADR 0047**: `quorra_scene::axis_aligned_rect` recognises
/// these four edges, so a *solid* fill of one goes down the analytic rectangle lane and
/// rasterises no coverage at all. A test that means the coverage lane needs a shape the
/// recogniser refuses instead — `m45.rs`'s `rasterised_rect_outline` is that shape, and it
/// exists because three tests once compared one lane with itself.
pub fn rect_outline(rect: Rect) -> Vec<Segment> {
    vec![
        Segment::MoveTo(rect.min),
        Segment::LineTo(Point::new(rect.max.x, rect.min.y)),
        Segment::LineTo(rect.max),
        Segment::LineTo(Point::new(rect.min.x, rect.max.y)),
        Segment::Close,
    ]
}

/// Opaque black.
pub fn black() -> Paint {
    Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0))
}

/// A dense page's shape: thousands of small rectangles. (5 933 is one dense page's
/// glyph count in the brief; rectangles stood in for glyphs until M4 and were never
/// replaced.)
///
/// **What this fixture is, and is not.** It exercises the analytic rectangle lane through
/// the command nothing sends: measured over the caller's 995-page corpus **not one page
/// emits a single `Command::Rect`** — every rectangle a real document draws arrives as a
/// `Fill` whose outline happens to be one (`doc/corpus-profile.md`). Since ADR 0047 such
/// a fill takes this same lane, so the lane below is no longer unused by documents; what
/// stays true is that they enter it by the other door, and that a page of nothing but
/// rectangles is a floor measurement rather than a page measurement. The page shapes
/// documents actually have are in `tests/archetypes.rs`, priced by counters instead of
/// clocks.
///
/// Two gates draw it — `perf_gate.rs` times it, `readback_cost.rs` counts what reading it
/// back allocates — and the second's stated byte count is a number about *this* page.
pub fn dense_scene() -> Scene {
    let mut builder = SceneBuilder::new();
    for i in 0..5_933_u32 {
        let x = f64::from(i % 80).mul_add(14.5, 3.25) as f32;
        let y = f64::from(i / 80).mul_add(15.25, 4.5) as f32;
        builder
            .rect(
                Rect::new(Point::new(x, y), Point::new(x + 9.75, y + 11.5)),
                Affine::IDENTITY,
                Color::new(0.1, 0.1, 0.1, 1.0),
                None,
                None,
            )
            .unwrap();
    }
    builder.finish()
}
