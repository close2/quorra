//! How many times a scene places each outline, at each scale and under each rule.
//!
//! The glyph atlas is a cache, and a cache is worth what it is *used* for. Whether it
//! will accept a tile is a question about the tile's size, which `AtlasStore::admits`
//! answers; whether accepting it buys anything is a question about the page, and only
//! the scene can answer that. ADR 0028 chose the coverage lane on the first question
//! alone and wrote down what that cost: on a page whose outlines are each placed once —
//! the caller's corpus median is 1.33 placements per distinct outline, so this is the
//! ordinary page and not a pathology — the atlas admits every tile, rasterises it,
//! uploads it, and reads it exactly once. ADR 0029 asks the second question here.
//!
//! # What this counts, and why it is an upper bound
//!
//! The atlas keys on the outline, the *linear* part of the device transform, the
//! sub-pixel phase and the fill rule. This census counts the first, second and fourth
//! and ignores the phase, so two placements it counts together may still be two atlas
//! keys — a glyph at the same size but a different sub-pixel offset is a different tile.
//!
//! That direction is deliberate. The count is an **upper bound on reuse**, so "this
//! outline is placed once" is a fact rather than a guess, and it is the only conclusion
//! the lane draws from it. Over-counting sends a tile to the atlas that might have been
//! faster on the device; under-counting would take a tile away from an atlas that would
//! have served it many times, which is the twenty-to-sixty-times mistake ADR 0028
//! measured. Between a bound that errs towards the cache and one that errs away from
//! it, this errs towards the cache.
//!
//! The phase could be counted too — it is a function of the translation, which is right
//! there in the command — but it would make the census depend on the viewport and the
//! quantum, and no measurement yet asks for the tighter number.

use crate::keyhash::FastMap;
use crate::raster::Rule;
use quorra_scene::{Command, FillRule, MaskDef, Paint, Scene};

/// What the census counts by: an outline drawn at one scale under one rule.
///
/// The linear part as bits rather than floats, because this is a hash key and `f32` is
/// not `Eq` — the same bits are the same transform, which is exactly the identity the
/// atlas key uses (`GlyphKey::linear`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Shape {
    outline: u32,
    linear: [u32; 4],
    rule: Rule,
}

/// How often each shape appears in one scene.
#[derive(Debug, Default)]
pub(crate) struct Census {
    counts: FastMap<Shape, u32>,
}

impl Census {
    /// Counts the scene's solid fills, including those inside groups and soft masks.
    ///
    /// Solid fills alone, because they are the only commands that reach the atlas: a
    /// stroke is expanded to a polygon and rasterised through the path lane, a shaded
    /// fill draws a quad over scratch coverage, and neither ever asks for a key.
    pub(crate) fn of(scene: &Scene) -> Self {
        let mut census = Self::default();
        census.walk(scene.commands());
        for mask in scene.masks() {
            let MaskDef { commands, .. } = mask;
            census.walk(commands);
        }
        census
    }

    fn walk(&mut self, commands: &[Command]) {
        for command in commands {
            match command {
                Command::Fill {
                    outline,
                    transform,
                    rule,
                    paint: Paint::Solid(_),
                    ..
                } => {
                    let shape = Shape {
                        outline: outline.0,
                        linear: [
                            transform.a.to_bits(),
                            transform.b.to_bits(),
                            transform.c.to_bits(),
                            transform.d.to_bits(),
                        ],
                        rule: match rule {
                            FillRule::NonZero => Rule::NonZero,
                            FillRule::EvenOdd => Rule::EvenOdd,
                        },
                    };
                    let count = self.counts.entry(shape).or_insert(0);
                    // A scene with four billion placements of one shape was refused by
                    // the frame budget long before it was counted; saturating says so
                    // rather than wrapping to "placed once" and losing the atlas.
                    *count = count.saturating_add(1);
                }
                Command::Group { commands, .. } => self.walk(commands),
                _ => {}
            }
        }
    }

    /// Whether this scene places this shape exactly once, so that an atlas entry for it
    /// would be written and read one time each.
    ///
    /// A shape the census never saw answers `false`: the census is the whole scene, so
    /// not finding a placement means the caller asked about something that is not a
    /// solid fill — and a lane must not be chosen on a count that was never taken.
    pub(crate) fn placed_once(&self, outline: u32, linear: [u32; 4], rule: Rule) -> bool {
        self.counts
            .get(&Shape {
                outline,
                linear,
                rule,
            })
            .is_some_and(|count| *count == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::Census;
    use crate::raster::Rule;
    use quorra_scene::{
        Affine, BlendMode, Color, Compose, FillRule, GroupSpec, LineCap, LineJoin, OutlineId,
        Paint, Scene, SceneBuilder, Stroke,
    };

    fn linear(transform: Affine) -> [u32; 4] {
        [
            transform.a.to_bits(),
            transform.b.to_bits(),
            transform.c.to_bits(),
            transform.d.to_bits(),
        ]
    }

    /// Every fill of one outline at one scale, wherever in the tree it sits.
    fn scene_with(build: impl FnOnce(&mut SceneBuilder, OutlineId)) -> Scene {
        let mut builder = SceneBuilder::new();
        // The census reads `OutlineId`s the scene was built with; no device is needed
        // to count them, which is the point of keeping this module device-free.
        let outline = OutlineId(7);
        build(&mut builder, outline);
        builder.finish()
    }

    fn fill(builder: &mut SceneBuilder, outline: OutlineId, transform: Affine) {
        builder
            .fill(
                outline,
                transform,
                FillRule::NonZero,
                Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0)),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .expect("a solid fill of an uploaded outline");
    }

    /// One placement is one placement; two at the same scale are not.
    #[test]
    fn a_shape_placed_twice_is_not_placed_once() {
        let scene = scene_with(|builder, outline| {
            fill(builder, outline, Affine::translate(0.0, 0.0));
            fill(builder, outline, Affine::translate(30.0, 0.0));
        });
        let census = Census::of(&scene);
        assert!(!census.placed_once(7, linear(Affine::IDENTITY), Rule::NonZero));
    }

    /// The same outline at two *scales* is two shapes, and each is placed once — which
    /// is the case the atlas key distinguishes and a count per outline would not.
    #[test]
    fn one_outline_at_two_scales_is_two_shapes_placed_once_each() {
        let scene = scene_with(|builder, outline| {
            fill(builder, outline, Affine::scale(1.0, 1.0));
            fill(builder, outline, Affine::scale(2.0, 2.0));
        });
        let census = Census::of(&scene);
        assert!(census.placed_once(7, linear(Affine::scale(1.0, 1.0)), Rule::NonZero));
        assert!(census.placed_once(7, linear(Affine::scale(2.0, 2.0)), Rule::NonZero));
    }

    /// A placement inside a group is a placement: the count is of the whole scene, and
    /// a group's children reach the same atlas the top level does.
    #[test]
    fn a_group_s_children_are_counted_with_everything_else() {
        let mut builder = SceneBuilder::new();
        let outline = OutlineId(7);
        fill(&mut builder, outline, Affine::IDENTITY);
        builder
            .group(
                GroupSpec {
                    alpha: 1.0,
                    blend: BlendMode::Normal,
                    clip: None,
                    knockout: false,
                    mask: None,
                    isolated: true,
                    compose: Compose::SrcOver,
                },
                |inner| {
                    fill(inner, outline, Affine::translate(10.0, 10.0));
                    Ok(())
                },
            )
            .expect("a group of one fill");
        let census = Census::of(&builder.finish());
        assert!(
            !census.placed_once(7, linear(Affine::IDENTITY), Rule::NonZero),
            "two placements, one of them nested"
        );
    }

    /// The two fill rules are two shapes: §8.5.3.3's rules give different pictures of
    /// the same outline, so the atlas keys on the rule and so does the census.
    #[test]
    fn the_fill_rule_separates_two_placements() {
        let mut builder = SceneBuilder::new();
        let outline = OutlineId(7);
        fill(&mut builder, outline, Affine::IDENTITY);
        builder
            .fill(
                outline,
                Affine::IDENTITY,
                FillRule::EvenOdd,
                Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0)),
                None,
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .expect("a solid fill");
        let census = Census::of(&builder.finish());
        assert!(census.placed_once(7, linear(Affine::IDENTITY), Rule::NonZero));
        assert!(census.placed_once(7, linear(Affine::IDENTITY), Rule::EvenOdd));
    }

    /// A stroke of the same outline is not a fill of it: strokes never reach the atlas,
    /// so counting them would keep a fill on the CPU lane for a placement that will
    /// never share its tile.
    #[test]
    fn a_stroke_is_not_a_placement() {
        let mut builder = SceneBuilder::new();
        let outline = OutlineId(7);
        fill(&mut builder, outline, Affine::IDENTITY);
        builder
            .stroke(
                outline,
                Affine::IDENTITY,
                Stroke {
                    width: 2.0,
                    adjust: false,
                    cap: LineCap::Butt,
                    join: LineJoin::Miter,
                    miter_limit: 4.0,
                },
                Paint::Solid(Color::new(0.0, 0.0, 0.0, 1.0)),
                None,
                BlendMode::Normal,
                None,
            )
            .expect("a stroke of an uploaded outline");
        let census = Census::of(&builder.finish());
        assert!(census.placed_once(7, linear(Affine::IDENTITY), Rule::NonZero));
    }

    /// A shape nobody placed is not "placed once".
    #[test]
    fn an_unseen_shape_is_not_placed_once() {
        let census = Census::of(&SceneBuilder::new().finish());
        assert!(!census.placed_once(7, linear(Affine::IDENTITY), Rule::NonZero));
    }
}
