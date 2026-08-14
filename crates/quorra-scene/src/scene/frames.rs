//! The open-frame stack: where a command lands, and how a nested body is discarded
//! whole.
//!
//! A builder is a stack of frames — one per open group or soft-mask body, the top one
//! taking every command appended. Three properties are the whole of this module, and
//! all three are why it is not just a `Vec::push`:
//!
//! - **The stack is popped on both paths**, so a body that refused leaves nothing
//!   behind and the builder stays usable. A half-built group is never a scene's
//!   content.
//! - **The stack is the depth bound.** §1.1 of the brief bounds the caller's display
//!   list at [`MAX_GROUP_DEPTH`], and the count that refuses is this one.
//! - **The stack carries the knockout question.** Whether a command lands inside a
//!   knockout group is a property of the frames above it, not of the command, and
//!   §11.6.5 makes a mask body start a fresh stack rather than inherit one.

use super::{Command, MAX_GROUP_DEPTH, SceneBuilder};
use crate::error::SceneError;

#[derive(Debug)]
pub(super) struct OpenFrame {
    commands: Vec<Command>,
    /// Whether the commands landing here sit inside a knockout group — the question
    /// [`NonIsolatedReason::InsideKnockoutGroup`](crate::error::NonIsolatedReason::InsideKnockoutGroup)
    /// asks. A soft mask's body starts a fresh stack: §11.6.5 renders the mask group on
    /// its own, so a knockout group outside it is not above the mask's content.
    inside_knockout: bool,
}

impl SceneBuilder {
    /// Run a nested body against its own command frame, popping it on both paths so
    /// an errored body is discarded whole and the builder stays consistent.
    ///
    /// `inside_knockout` is what the *body's* commands are nested in, which is why the
    /// group's own knockout flag is folded in by the caller and a mask body passes
    /// `false`.
    pub(super) fn nested_body(
        &mut self,
        inside_knockout: bool,
        body: impl FnOnce(&mut Self) -> Result<(), SceneError>,
    ) -> Result<Vec<Command>, SceneError> {
        if self.open_frames.len() >= MAX_GROUP_DEPTH {
            return Err(SceneError::GroupTooDeep {
                limit: MAX_GROUP_DEPTH,
            });
        }
        self.open_frames.push(OpenFrame {
            commands: Vec::new(),
            inside_knockout,
        });
        let body_result = body(self);
        let finished = self.open_frames.pop().unwrap_or(OpenFrame {
            commands: Vec::new(),
            inside_knockout,
        });
        body_result?;
        Ok(finished.commands)
    }

    /// Whether commands appended right now land inside a knockout group.
    pub(super) fn inside_knockout(&self) -> bool {
        self.open_frames
            .last()
            .is_some_and(|frame| frame.inside_knockout)
    }

    pub(super) fn push(&mut self, command: Command) {
        match self.open_frames.last_mut() {
            Some(frame) => frame.commands.push(command),
            None => self.commands.push(command),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::blend::{BlendMode, Compose};
    use crate::error::SceneError;
    use crate::geom::{Affine, Point, Rect};
    use crate::paint::Color;
    use crate::scene::fixtures::{black, plain_group, unit_rect};
    use crate::scene::{Command, GroupSpec, MAX_GROUP_DEPTH, SceneBuilder};

    /// Nesting to the bound succeeds; one deeper is refused with the bound named, and
    /// the builder stays usable afterwards.
    #[test]
    fn group_depth_is_bounded_at_sixteen() {
        fn nest(builder: &mut SceneBuilder, remaining: usize) -> Result<(), SceneError> {
            if remaining == 0 {
                return builder.rect(
                    Rect::new(Point::new(0.0, 0.0), Point::new(1.0, 1.0)),
                    Affine::IDENTITY,
                    Color::new(0.0, 0.0, 0.0, 1.0),
                    None,
                    None,
                );
            }
            builder.group(
                GroupSpec {
                    alpha: 1.0,
                    blend: BlendMode::Normal,
                    clip: None,
                    knockout: false,
                    mask: None,
                    isolated: true,
                    compose: Compose::SrcOver,
                },
                |b| nest(b, remaining - 1),
            )
        }

        let mut builder = SceneBuilder::new();
        nest(&mut builder, MAX_GROUP_DEPTH).expect("the bound itself is allowed");

        let mut too_deep = SceneBuilder::new();
        assert!(matches!(
            nest(&mut too_deep, MAX_GROUP_DEPTH + 1),
            Err(SceneError::GroupTooDeep {
                limit: MAX_GROUP_DEPTH
            })
        ));
        too_deep
            .rect(unit_rect(), Affine::IDENTITY, black(), None, None)
            .expect("the builder survives a refused group");
        let scene = builder.finish();
        assert_eq!(scene.cost().group_depth, MAX_GROUP_DEPTH);
        assert_eq!(scene.cost().commands, MAX_GROUP_DEPTH + 1);
    }

    /// An error inside a group discards the group whole — no half-built group is ever
    /// a scene's content — and propagates to the caller.
    #[test]
    fn errored_groups_are_discarded_whole() {
        let mut builder = SceneBuilder::new();
        let result = builder.group(plain_group(), |b| {
            b.rect(unit_rect(), Affine::IDENTITY, black(), None, None)?;
            b.rect(
                unit_rect(),
                Affine::IDENTITY,
                Color::new(2.0, 0.0, 0.0, 1.0),
                None,
                None,
            )
        });
        assert!(matches!(result, Err(SceneError::InvalidColor(_))));
        assert!(
            builder.finish().commands().is_empty(),
            "a group that errored must not appear in the scene"
        );
    }

    /// Commands land in their group, groups nest, and the whole shape comes back.
    #[test]
    fn groups_nest_and_round_trip() {
        let mut builder = SceneBuilder::new();
        builder
            .group(
                GroupSpec {
                    alpha: 0.5,
                    ..plain_group()
                },
                |b| {
                    b.rect(unit_rect(), Affine::IDENTITY, black(), None, None)?;
                    b.group(plain_group(), |inner| {
                        inner.rect(unit_rect(), Affine::IDENTITY, black(), None, None)
                    })
                },
            )
            .expect("valid nested groups");
        let scene = builder.finish();
        assert_eq!(scene.commands().len(), 1);
        let Command::Group { spec, commands } = &scene.commands()[0] else {
            panic!("expected a group at the top level");
        };
        assert!((spec.alpha - 0.5).abs() < f32::EPSILON);
        assert_eq!(commands.len(), 2);
        assert!(matches!(commands[1], Command::Group { .. }));
        // A group is itself a command: outer group + rect + inner group + inner rect.
        assert_eq!(scene.cost().commands, 4);
        assert_eq!(scene.cost().group_depth, 2);
        assert!(scene.cost().retained_bytes > 0);
    }
}
