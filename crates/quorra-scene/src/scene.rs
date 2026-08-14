//! The scene: an immutable, device-independent description of marks.
//!
//! This is the centre of the library's design, and `doc/RENDER_LIBRARY.md` §2.3 states
//! the property that decides it:
//!
//! > The single most important property in this document: a `Scene` must contain no
//! > reference to a viewport, a resolution, a device transform, or a target size.
//!
//! Zoom, scroll, window resize and tiled output are all *the same scene at a different
//! viewport*. If building a scene were a function of the target, every zoom step would
//! redo it — and encoding measured 1.1–1.6 ms, flat across a sixteenfold range of
//! resolutions, which is 22% of a thumbnail's frame and 1.5 ms per frame the caller's
//! interpreter is not getting.
//!
//! The corollary, which §2.3 asks to have stated in our documentation rather than left
//! implicit: **a [`Scene`] is `Send + Sync`, cheap to clone, and building one requires
//! no device.** In this crate that is structural rather than aspirational — there is no
//! device type in scope to require. See `doc/adr/0001`.
//!
//! # The four parts, and where each one's rules live
//!
//! This file holds the finished scene and nothing else; the parts that make one are
//! private modules re-exported here, so that every path a caller has ever written keeps
//! working while each rule has one place to be read:
//!
//! - `command` — the vocabulary a scene is written in: [`Command`] and the definitions
//!   it points at. The clause citations for what a mark *means* are there.
//! - `builder` — [`SceneBuilder`]: one method per command, and nothing but the three
//!   steps each of them takes.
//! - `frames` — the open-frame stack a nested body runs against, which is also the
//!   depth bound and the knockout question.
//! - `validate` — §4.7's refusals, all of them, in the order the boundary applies them.
//! - `cost` — [`Cost`] and the walk that measures it, run once at
//!   [`SceneBuilder::finish`].
//!
//! # State: M6
//!
//! The vocabulary is the brief's §2.3 minus M7's images: `fill`, `stroke`, `rect`,
//! `clip`, `group` and `mask` all exist. One deliberate divergence from the brief's
//! illustrative signatures, recorded here and in `doc/PLAN.md` integration note 8:
//! the `mask` parameter comes **last** in each builder method rather than beside
//! `clip`, so that growing the vocabulary was a mechanical widening for every call
//! site. A scene may *hold* every command; what a device cannot yet *draw* (M7's
//! images) is refused loudly at render time, never approximated (§5).
//!
//! # What a `Scene` may not become
//!
//! Not a scene graph, not a retained widget tree, no animation and no timeline (§9). It
//! is built by an interpreter and thrown away when the page changes. §11.5 asks what
//! one costs to hold, against a target of a dozen resident pages out of a 1 023-page
//! document — [`Scene::cost`] is the running answer.

use std::sync::Arc;

mod builder;
mod command;
mod cost;
mod frames;
mod validate;

#[cfg(test)]
mod fixtures;

pub use builder::SceneBuilder;
pub use command::{ClipDef, Command, GroupSpec, ImageFilter, MAX_GROUP_DEPTH, MaskDef};
pub use cost::Cost;
pub use validate::MAX_COORDINATE;

/// What is to be drawn: an immutable, device-independent description of marks.
///
/// `Send + Sync`, cheap to clone (an `Arc` inside), and containing no reference to a
/// viewport, a resolution, a device transform or a target size — the brief's §2.3, held
/// structurally (`doc/adr/0001`). A blank scene is a legitimate scene and renders to a
/// legitimate, empty frame (§5).
#[derive(Debug, Clone)]
pub struct Scene {
    data: Arc<SceneData>,
}

#[derive(Debug)]
struct SceneData {
    commands: Vec<Command>,
    clips: Vec<ClipDef>,
    masks: Vec<MaskDef>,
    cost: Cost,
}

impl Scene {
    /// The top-level commands, in the order the builder received them.
    ///
    /// A device may draw them in any order whose result is identical (§4.6).
    #[must_use]
    pub fn commands(&self) -> &[Command] {
        &self.data.commands
    }

    /// The clip definitions, indexed by [`ClipId`](crate::ids::ClipId). A chain is
    /// resolved by following `parent` links; every link is valid by construction (the
    /// builder refused anything else).
    #[must_use]
    pub fn clips(&self) -> &[ClipDef] {
        &self.data.clips
    }

    /// The soft-mask definitions, indexed by [`MaskId`](crate::ids::MaskId). A mask's
    /// own commands may reference only masks defined before it, so realisation in index
    /// order is always possible.
    #[must_use]
    pub fn masks(&self) -> &[MaskDef] {
        &self.data.masks
    }

    /// What this scene costs, comparable against `Device::limits` before a frame is
    /// attempted. Computed once, at [`SceneBuilder::finish`]; asking is free.
    #[must_use]
    pub fn cost(&self) -> Cost {
        self.data.cost
    }
}

#[cfg(test)]
mod tests {
    use super::{Scene, SceneBuilder};
    use crate::geom::Affine;
    use crate::scene::fixtures::{black, unit_rect};

    /// §2.3's corollary, checked by the compiler: the day someone adds an `Rc` this
    /// stops compiling, rather than the caller's worker thread failing.
    #[test]
    fn scene_is_send_sync_and_static() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<Scene>();
    }

    /// A blank scene is a legitimate scene (§5): zero commands, zero cost, no error.
    #[test]
    fn blank_scene_is_legitimate() {
        let scene = SceneBuilder::new().finish();
        assert!(scene.commands().is_empty());
        assert_eq!(scene.cost().commands, 0);
    }

    /// Clones share the same data rather than copying it — "cheap to clone (an `Arc`
    /// inside)" is the brief's own phrasing and this pins it.
    #[test]
    fn clones_share_storage() {
        let mut builder = SceneBuilder::new();
        builder
            .rect(unit_rect(), Affine::IDENTITY, black(), None, None)
            .expect("valid input");
        let scene = builder.finish();
        let clone = scene.clone();
        assert_eq!(
            scene.commands().as_ptr(),
            clone.commands().as_ptr(),
            "a clone must reference the same command storage"
        );
    }
}
