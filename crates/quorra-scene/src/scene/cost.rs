//! What a scene costs, and the walk that counts it.
//!
//! §11.5 of the brief asks what a scene costs to *hold*, against a target of a dozen
//! resident pages out of a 1 023-page document, and §5 asks that a limit be
//! discoverable before a frame rather than after one. Both are answered by the same
//! walk, run once at [`SceneBuilder::finish`](super::SceneBuilder::finish) — which is
//! why it lives here rather than in the builder that calls it or the scene that
//! carries the answer.

use super::{ClipDef, Command, MaskDef, SceneData};

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
}

/// The cost walk: commands and depth through nesting (mask bodies included),
/// plus a byte estimate of what the scene retains (§11.5).
pub(super) fn measure(commands: &[Command], clips: &[ClipDef], masks: &[MaskDef]) -> Cost {
    fn walk(commands: &[Command], depth: usize, cost: &mut Cost) {
        cost.group_depth = cost.group_depth.max(depth);
        for command in commands {
            cost.commands = cost.commands.saturating_add(1);
            cost.retained_bytes = cost.retained_bytes.saturating_add(size_of::<Command>());
            if let Command::Group { commands, .. } = command {
                walk(commands, depth.saturating_add(1), cost);
            }
        }
    }
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
    walk(commands, 0, &mut cost);
    for mask in masks {
        walk(&mask.commands, 1, &mut cost);
    }
    cost
}
