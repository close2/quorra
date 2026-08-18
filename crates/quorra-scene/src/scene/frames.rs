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
//! - **The stack carries the knockout question, and it is two questions.** Whether a
//!   command lands inside a knockout group is a property of the frames above it, not of
//!   the command, and §11.6.5 makes a mask body start a fresh stack rather than inherit
//!   one. [`Knockout`] says why one boolean is not enough.

use super::{Command, MAX_GROUP_DEPTH, SceneBuilder};
use crate::error::SceneError;

/// What §11.4.6 makes of the commands landing in one frame.
///
/// **Two questions, and they have different answers**, which is why this is a pair rather
/// than a flag. §11.4.6 governs the *elements* of a knockout group — the commands one
/// level down — and a group two levels down is an element of its own parent, which
/// composites it by §11.3.6 like any other. So a rule about "an element of a knockout
/// group" reads [`Knockout::element`] and a rule about "anywhere below one" reads
/// [`Knockout::inside`], and reading the wrong one is a refusal in the wrong place: the
/// caller's expansion of §11.4.6 writes each half as a group whose own elements may be
/// groups (ADR 0069), and that construction is correct at every depth.
#[derive(Debug, Clone, Copy)]
pub(super) struct Knockout {
    /// The group *immediately* enclosing these commands is a knockout group, so §11.4.6
    /// weights each of them by its own source shape.
    pub element: bool,
    /// Some group enclosing these commands, at any depth, is a knockout group — the
    /// question
    /// [`NonIsolatedReason::InsideKnockoutGroup`](crate::error::NonIsolatedReason::InsideKnockoutGroup)
    /// asks.
    pub inside: bool,
}

impl Knockout {
    /// The page's own context, and a soft mask's: §11.6.5 renders the mask group on its
    /// own, so a knockout group outside a `mask()` call is not above the mask's content.
    pub(super) const NONE: Self = Self {
        element: false,
        inside: false,
    };
}

#[derive(Debug)]
pub(super) struct OpenFrame {
    commands: Vec<Command>,
    /// What the commands landing here are nested in (§11.4.6).
    knockout: Knockout,
}

impl SceneBuilder {
    /// Run a nested body against its own command frame, popping it on both paths so
    /// an errored body is discarded whole and the builder stays consistent.
    ///
    /// `knockout` describes what the *body's* commands are nested in, which is why the
    /// group's own knockout flag is folded in by the caller and a mask body passes
    /// [`Knockout::NONE`].
    pub(super) fn nested_body(
        &mut self,
        knockout: Knockout,
        body: impl FnOnce(&mut Self) -> Result<(), SceneError>,
    ) -> Result<Vec<Command>, SceneError> {
        if self.open_frames.len() >= MAX_GROUP_DEPTH {
            return Err(SceneError::GroupTooDeep {
                limit: MAX_GROUP_DEPTH,
            });
        }
        self.open_frames.push(OpenFrame {
            commands: Vec::new(),
            knockout,
        });
        let body_result = body(self);
        let finished = self.open_frames.pop().unwrap_or(OpenFrame {
            commands: Vec::new(),
            knockout,
        });
        body_result?;
        Ok(finished.commands)
    }

    /// Whether commands appended right now land inside a knockout group, at any depth.
    pub(super) fn inside_knockout(&self) -> bool {
        self.open_frames
            .last()
            .is_some_and(|frame| frame.knockout.inside)
    }

    /// Whether commands appended right now are *elements* of a knockout group — the
    /// group immediately enclosing them is one, so §11.4.6 weights each by its own
    /// source shape.
    pub(super) fn element_of_knockout(&self) -> bool {
        self.open_frames
            .last()
            .is_some_and(|frame| frame.knockout.element)
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

    /// The stack's two knockout answers diverge, and this is the module that owes that
    /// property: [`SceneBuilder::inside_knockout`] accumulates down the stack while
    /// [`SceneBuilder::element_of_knockout`] asks only about the frame's own group.
    ///
    /// Asked here rather than through a refusal so that the divergence is stated once, at
    /// the stack, instead of being inferred from which error a scene got.
    #[test]
    fn the_two_knockout_questions_are_asked_of_different_frames() {
        let mut builder = SceneBuilder::new();
        // The page: neither.
        assert!(!builder.inside_knockout());
        assert!(!builder.element_of_knockout());

        let knockout = GroupSpec {
            knockout: true,
            ..plain_group()
        };
        builder
            .group(knockout, |body| {
                // A direct element of the knockout group: both.
                assert!(body.inside_knockout());
                assert!(body.element_of_knockout());
                // §11.4.6's stages are the way a group sits here (ADR 0069); inside one,
                // the commands are elements of an *ordinary* group and only `inside`
                // survives.
                body.group(
                    GroupSpec {
                        compose: Compose::DestOut,
                        ..plain_group()
                    },
                    |half| {
                        assert!(half.inside_knockout());
                        assert!(!half.element_of_knockout());
                        // And one level deeper still, so the answer does not oscillate.
                        half.group(plain_group(), |deeper| {
                            assert!(deeper.inside_knockout());
                            assert!(!deeper.element_of_knockout());
                            Ok(())
                        })
                    },
                )?;
                // A soft mask's body starts a fresh stack: §11.6.5 renders the mask group
                // on its own, so neither answer reaches into it.
                body.mask(crate::mask::MaskKind::Alpha, None, |mask_body| {
                    assert!(!mask_body.inside_knockout());
                    assert!(!mask_body.element_of_knockout());
                    Ok(())
                })
                .map(|_| ())
            })
            .expect("every group in this scene is one the builder accepts");

        // And the stack unwound: the page's answers are back.
        assert!(!builder.inside_knockout());
        assert!(!builder.element_of_knockout());
    }

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
