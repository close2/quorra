//! What a scene costs, and the walk that counts it.
//!
//! §11.5 of the brief asks what a scene costs to *hold*, against a target of a dozen
//! resident pages out of a 1 023-page document, and §5 asks that a limit be
//! discoverable before a frame rather than after one. Both are answered by the same
//! walk, run once at [`SceneBuilder::finish`](super::SceneBuilder::finish) — which is
//! why it lives here rather than in the builder that calls it or the scene that
//! carries the answer.

use std::collections::HashSet;
use std::sync::Arc;

use super::{ClipDef, Command, MaskDef, SceneData};
use crate::function::{FnOp, FunctionPaint};
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
    /// How many **distinct** ISO 32000-2 §7.10.5 programs the scene's
    /// [`Paint::Function`]s reference.
    ///
    /// Distinct, not referencing: CLAUDE.md's rule is to instrument the count of distinct
    /// keys rather than the hit rate, and this is the number that matters twice over —
    /// each distinct program is one shader a device must generate and compile (6.3 ms
    /// cold in `doc/spike-function-paint.md`), and one program allocation the scene
    /// retains however many marks paint with it. A thousand fills sharing one `Arc` count
    /// one here.
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
    // The set is the walk's memory of which programs it has already paid for; it is the
    // only state the recursion carries that is not the cost itself.
    let mut counted = HashSet::new();
    walk(commands, 0, &mut counted, &mut cost);
    for mask in masks {
        walk(&mask.commands, 1, &mut counted, &mut cost);
    }
    cost
}

fn walk(commands: &[Command], depth: usize, counted: &mut HashSet<usize>, cost: &mut Cost) {
    cost.group_depth = cost.group_depth.max(depth);
    for command in commands {
        cost.commands = cost.commands.saturating_add(1);
        cost.retained_bytes = cost.retained_bytes.saturating_add(size_of::<Command>());
        match command {
            Command::Group { commands, .. } => {
                walk(commands, depth.saturating_add(1), counted, cost);
            }
            Command::Fill { paint, .. } | Command::Stroke { paint, .. } => {
                account_paint(paint, counted, cost);
            }
            Command::Rect { .. } | Command::Image { .. } => {}
        }
    }
}

/// What a paint adds to the scene's retained bytes beyond the command that names it.
///
/// Only [`Paint::Function`] has anything to add: every other paint is a fixed handful of
/// bytes already counted inside [`Command`], and its resources (ramps, meshes, images)
/// belong to a device rather than to the scene.
///
/// Sharing is counted by `Arc` identity, which is what "retained" means: two structurally
/// equal programs behind two separate allocations really are two allocations, and one
/// program behind one `Arc` is one however many marks reach it.
fn account_paint(paint: &Paint, counted: &mut HashSet<usize>, cost: &mut Cost) {
    let Paint::Function(function) = paint else {
        return;
    };
    let identity = Arc::as_ptr(function) as usize;
    if !counted.insert(identity) {
        return;
    }
    cost.function_programs = cost.function_programs.saturating_add(1);
    cost.retained_bytes = cost
        .retained_bytes
        .saturating_add(size_of::<FunctionPaint>())
        .saturating_add(function.program.len().saturating_mul(size_of::<FnOp>()));
}

#[cfg(test)]
mod tests {
    use super::{Cost, FnOp, FunctionPaint};
    use crate::blend::{BlendMode, Compose, FillRule};
    use crate::geom::Affine;
    use crate::ids::OutlineId;
    use crate::paint::Paint;
    use crate::scene::fixtures::function_paint;
    use crate::scene::{Scene, SceneBuilder};
    use std::sync::Arc;

    /// Fill `count` times with `paint`, and hand back what the scene says it costs.
    fn scene_filled_with(paint: &Paint, count: usize) -> Scene {
        let mut builder = SceneBuilder::new();
        for _ in 0..count {
            builder
                .fill(
                    OutlineId(0),
                    Affine::IDENTITY,
                    FillRule::NonZero,
                    paint.clone(),
                    None,
                    BlendMode::Normal,
                    Compose::SrcOver,
                    None,
                )
                .expect("a well-formed function paint");
        }
        builder.finish()
    }

    /// What one command of the scene's own shape costs, so a test can subtract it and
    /// talk about the program alone.
    fn baseline(count: usize) -> Cost {
        scene_filled_with(&Paint::Solid(crate::scene::fixtures::black()), count).cost()
    }

    /// §11.5's question is what a scene costs to *hold*, and a program held once is held
    /// once however many marks paint with it. Counting per command would report a
    /// thousand-mark page as holding a thousand programs it does not hold.
    #[test]
    fn one_shared_program_is_counted_once_however_many_marks_reach_it() {
        let paint = Paint::Function(Arc::new(function_paint(64)));
        let cost = scene_filled_with(&paint, 100).cost();
        assert_eq!(cost.commands, 100);
        assert_eq!(cost.function_programs, 1);
        let program_bytes =
            cost.retained_bytes - baseline(100).retained_bytes - size_of::<FunctionPaint>();
        assert_eq!(program_bytes, 64 * size_of::<FnOp>());
    }

    /// Two allocations really are two allocations, even when their instructions match:
    /// `Arc` identity is what "retained" means, and the count is of distinct programs
    /// rather than of distinct instruction lists.
    #[test]
    fn two_separately_allocated_programs_are_counted_twice() {
        let mut builder = SceneBuilder::new();
        for _ in 0..2 {
            builder
                .fill(
                    OutlineId(0),
                    Affine::IDENTITY,
                    FillRule::NonZero,
                    Paint::Function(Arc::new(function_paint(8))),
                    None,
                    BlendMode::Normal,
                    Compose::SrcOver,
                    None,
                )
                .expect("a well-formed function paint");
        }
        let cost = builder.finish().cost();
        assert_eq!(cost.function_programs, 2);
        let program_bytes = cost.retained_bytes - baseline(2).retained_bytes;
        assert_eq!(
            program_bytes,
            2 * (size_of::<FunctionPaint>() + 8 * size_of::<FnOp>())
        );
    }

    /// A program inside a soft mask's body is as retained as one on the page, and the
    /// walk that finds it is the same walk.
    #[test]
    fn a_program_inside_a_mask_body_is_counted_too() {
        let shared = Arc::new(FunctionPaint {
            program: Arc::from([FnOp::PushReal(1.0)].as_slice()),
            ..function_paint(1)
        });
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
                        Paint::Function(Arc::clone(&shared)),
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
        assert_eq!(baseline(3).function_programs, 0);
    }
}
