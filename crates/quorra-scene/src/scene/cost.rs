//! What a scene costs, and the walk that counts it.
//!
//! §11.5 of the brief asks what a scene costs to *hold*, against a target of a dozen
//! resident pages out of a 1 023-page document, and §5 asks that a limit be
//! discoverable before a frame rather than after one. Both are answered by the same
//! walk, run once at [`SceneBuilder::finish`](super::SceneBuilder::finish) — which is
//! why it lives here rather than in the builder that calls it or the scene that
//! carries the answer.

use std::collections::HashSet;

use super::{ClipDef, Command, MaskDef, SceneData};
use crate::ids::FunctionId;
use crate::paint::Paint;

/// What a scene costs, so that a limit can be discovered *before* a frame (§5's second
/// preference): a caller compares this against `Device::limits` and falls back rather
/// than discovering a refusal afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cost {
    /// Number of drawing commands, counted through group nesting.
    pub commands: usize,
    /// Number of clip regions the scene defines.
    pub clips: usize,
    /// Number of soft masks the scene defines.
    pub masks: usize,
    /// Deepest group nesting the scene reaches.
    pub group_depth: usize,
    /// Heap bytes the scene retains while held — §11.5's question, measured per scene.
    pub retained_bytes: usize,
    /// How many **distinct** [`FunctionId`]s the scene's
    /// [`Paint::Function`](crate::paint::Paint::Function)s reference.
    ///
    /// Distinct, not referencing: CLAUDE.md's rule is to instrument the count of distinct
    /// keys rather than the hit rate, and this is exactly the number a device pays for —
    /// **one generated shader per distinct program**, 6.3 ms of cold pipeline compile
    /// each (`doc/spike-function-paint.md`). A thousand fills sharing one identifier
    /// compile one shader and count one here; a hundred fills naming a hundred
    /// identifiers is a hundred, and that is the page a caller wants to hear about before
    /// the frame rather than during it.
    ///
    /// The programs themselves are **not** in [`Cost::retained_bytes`]: they live on a
    /// device (§2.2), and what a scene holds is four bytes of handle per reference,
    /// already counted inside [`Command`].
    pub function_programs: usize,
}

/// The cost walk: commands and depth through nesting (mask bodies included),
/// plus a byte estimate of what the scene retains (§11.5).
pub(super) fn measure(commands: &[Command], clips: &[ClipDef], masks: &[MaskDef]) -> Cost {
    let mut cost = Cost {
        clips: clips.len(),
        masks: masks.len(),
        retained_bytes: clips
            .len()
            .saturating_mul(size_of::<ClipDef>())
            .saturating_add(masks.len().saturating_mul(size_of::<MaskDef>()))
            .saturating_add(size_of::<SceneData>()),
        ..Cost::default()
    };
    // The set is the walk's memory of which programs it has already counted; it is the
    // only state the recursion carries that is not the cost itself.
    let mut seen = HashSet::new();
    walk(commands, 0, &mut seen, &mut cost);
    for mask in masks {
        walk(&mask.commands, 1, &mut seen, &mut cost);
    }
    cost
}

fn walk(commands: &[Command], depth: usize, seen: &mut HashSet<FunctionId>, cost: &mut Cost) {
    cost.group_depth = cost.group_depth.max(depth);
    for command in commands {
        cost.commands = cost.commands.saturating_add(1);
        cost.retained_bytes = cost.retained_bytes.saturating_add(size_of::<Command>());
        match command {
            Command::Group { commands, .. } => {
                walk(commands, depth.saturating_add(1), seen, cost);
            }
            Command::Fill { paint, .. } | Command::Stroke { paint, .. } => {
                if let Paint::Function { program, .. } = paint
                    && seen.insert(*program)
                {
                    cost.function_programs = cost.function_programs.saturating_add(1);
                }
            }
            Command::Rect { .. } | Command::Image { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::blend::{BlendMode, Compose, FillRule};
    use crate::geom::Affine;
    use crate::ids::{FunctionId, OutlineId};
    use crate::paint::Paint;
    use crate::scene::fixtures::{black, function_paint};
    use crate::scene::{Scene, SceneBuilder};

    /// Fill `count` times with `paint`, and hand back the scene.
    fn scene_filled_with(paint: Paint, count: usize) -> Scene {
        let mut builder = SceneBuilder::new();
        for _ in 0..count {
            builder
                .fill(
                    OutlineId(0),
                    Affine::IDENTITY,
                    FillRule::NonZero,
                    paint,
                    None,
                    BlendMode::Normal,
                    Compose::SrcOver,
                    None,
                )
                .expect("a well-formed paint");
        }
        builder.finish()
    }

    /// The number a device pays for is the count of distinct programs, because that is
    /// the count of shaders it generates. Counting per command would report a
    /// thousand-mark page as a thousand compilations it will never do.
    #[test]
    fn one_shared_program_is_counted_once_however_many_marks_reach_it() {
        let cost = scene_filled_with(function_paint(FunctionId(4)), 100).cost();
        assert_eq!(cost.commands, 100);
        assert_eq!(cost.function_programs, 1);
    }

    /// Two identifiers are two programs and two generated shaders, whatever their
    /// instructions turn out to be — a scene cannot know, and the device's own cache is
    /// what answers that question later.
    #[test]
    fn two_distinct_identifiers_are_counted_twice() {
        let mut builder = SceneBuilder::new();
        for id in [FunctionId(1), FunctionId(2)] {
            builder
                .fill(
                    OutlineId(0),
                    Affine::IDENTITY,
                    FillRule::NonZero,
                    function_paint(id),
                    None,
                    BlendMode::Normal,
                    Compose::SrcOver,
                    None,
                )
                .expect("a well-formed function paint");
        }
        assert_eq!(builder.finish().cost().function_programs, 2);
    }

    /// A program referenced from inside a soft mask's body is as much a compilation as
    /// one on the page, and the walk that finds it is the same walk.
    #[test]
    fn a_program_inside_a_mask_body_is_counted_too() {
        let mut builder = SceneBuilder::new();
        builder
            .mask(
                crate::mask::MaskKind::Alpha,
                None,
                |inner: &mut SceneBuilder| {
                    inner.fill(
                        OutlineId(0),
                        Affine::IDENTITY,
                        FillRule::NonZero,
                        function_paint(FunctionId(9)),
                        None,
                        BlendMode::Normal,
                        Compose::SrcOver,
                        None,
                    )
                },
            )
            .expect("a mask whose body paints with a function");
        assert_eq!(builder.finish().cost().function_programs, 1);
    }

    /// A scene with no function paint reports none — the field is a count of what is
    /// there, not a flag that something might be.
    #[test]
    fn a_scene_without_a_function_paint_reports_no_programs() {
        let cost = scene_filled_with(Paint::Solid(black()), 3).cost();
        assert_eq!(cost.function_programs, 0);
    }
}
