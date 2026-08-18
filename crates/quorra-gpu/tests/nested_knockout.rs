//! A group used as an element of a knockout group: refused, because §11.4.6 wants a
//! shape this raster does not carry.
//!
//! # Where the expected values come from
//!
//! ISO 32000-2 §11.4.6 names this element kind and puts an obligation on it:
//!
//! > The separate shape value shall be computed in any group that is subsequently used as
//! > an element of a knockout group.
//!
//! and §11.3.7.2 says what that value is:
//!
//! > The shape of a group object shall be the union (as defined in 11.3.7.3, "Result
//! > shape and opacity") of the shapes of the objects it contains.
//!
//! A finished group reaches the compositor as **one** premultiplied texture. Its alpha
//! is §11.3.7.3's result alpha — the union of each element's shape *times* its opacity —
//! so the two quantities §11.4.6 weights apart arrive as one number, and the union of
//! shapes is not recoverable from it. §11.4.6's own line, which `common::clause` writes
//! once, needs both:
//!
//! > 𝛼gi = (1 − 𝑓si) × 𝛼gi−1 + 𝑓si × 𝛼t
//!
//! `f` is the shape, and `𝛼t` is the element composited with the group's initial backdrop
//! — transparent in an isolated group (§11.4.5), where §11.3.6 leaves `co = as·Cs`. So the
//! line is `P' = (1 − f) × P + S`, premultiplied.
//!
//! **What is refused, and what is not.** [`quorra_scene::Compose::DestOut`] then
//! [`quorra_scene::Compose::Plus`] on two groups *is* that line, asked for by name
//! (ADR 0033): the caller draws the shape half as content it knows to be opaque, and the
//! library draws the two stages. That construction is accepted here at every depth, which
//! is why the refusal reads "an element of a knockout group" and not "anywhere below one"
//! — the caller's own expansion makes a group's stated shape the group of its elements'
//! stated shapes, so the halves themselves hold groups.
//!
//! **Why an ordinary group stands in for the refused one below.** `encode_group` builds
//! its `ChildOp` without consulting the enclosing `DrawStyle`, and a `Compose::SrcOver`
//! group maps to the same `compose: 0` in either position — so the refused construction
//! and the same content inside an *ordinary* group produced byte-identical frames
//! (`[128, 76, 128, 255]` against `[26, 102, 229, 128]`, measured at 16 × 16 over an
//! opaque cover before this refusal existed). The ordinary group is therefore not an
//! analogy: it is the same encode, and measuring it is measuring what the refusal spares
//! the caller.

// Test-file lint policy as in m1.rs; the reference math mirrors clause arithmetic.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    // Pixel indexing and the clause's own arithmetic, over rasters this file just drew.
    clippy::arithmetic_side_effects
)]

use quorra_gpu::Device;
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, GroupSpec, MaskKind, OutlineId, Paint, Point,
    Rect, SceneBuilder, SceneError, Segment,
};

mod common;

use common::clause::deviation_from_the_clause;
use common::headless::{device, render};

const SIZE: u32 = 64;

/// The opaque content the nested group lands on inside the outer group, so its immediate
/// backdrop is opaque while the group's initial backdrop is still transparent — which is
/// the whole difference between §11.4.6's weighted average and §11.3.6's composite.
const UNDER: Color = Color {
    r: 0.9,
    g: 0.2,
    b: 0.1,
    a: 1.0,
};

/// What the nested group paints. Opaque, so the group's *shape* is its wedge and its
/// *opacity* is the group's constant alpha alone — the two numbers the clause reads apart.
const OBJECT: Color = Color {
    r: 0.1,
    g: 0.4,
    b: 0.9,
    a: 1.0,
};

/// The nested group's constant alpha. Not 1: at opacity 1 the two readings coincide
/// exactly — `(1 − f)·P` and `(1 − f·q)·P` are the same term — so a fixture that left it
/// at 1 would hold nothing.
const NESTED_ALPHA: f32 = 0.5;

/// A triangle with two diagonal edges, so the nested group's shape has partially covered
/// pixels: a shape that is only ever 0 or 1 makes a weighted average and a composite agree
/// for the wrong reason.
fn wedge(device: &mut Device) -> OutlineId {
    device
        .upload_outline(&[
            Segment::MoveTo(Point::new(10.0, 10.0)),
            Segment::LineTo(Point::new(54.0, 10.0)),
            Segment::LineTo(Point::new(10.0, 54.0)),
            Segment::Close,
        ])
        .unwrap()
}

fn full() -> Rect {
    Rect::new(Point::new(0.0, 0.0), Point::new(SIZE as f32, SIZE as f32))
}

/// An isolated group: the two flags this file varies, and nothing else.
fn group(knockout: bool, alpha: f32, compose: Compose) -> GroupSpec {
    GroupSpec {
        alpha,
        blend: BlendMode::Normal,
        clip: None,
        knockout,
        mask: None,
        isolated: true,
        compose,
    }
}

/// The full-target opaque cover the outer group holds before the nested group.
fn cover(builder: &mut SceneBuilder) -> Result<(), SceneError> {
    builder.rect(full(), Affine::IDENTITY, UNDER, None, None)
}

/// The wedge, filled solid at `colour`.
fn wedge_fill(
    builder: &mut SceneBuilder,
    outline: OutlineId,
    colour: Color,
) -> Result<(), SceneError> {
    builder.fill(
        outline,
        Affine::IDENTITY,
        FillRule::NonZero,
        Paint::Solid(colour),
        None,
        BlendMode::Normal,
        Compose::SrcOver,
        None,
    )
}

// ------------------------------------------------------------------ the refusal

/// One shape of nested group, built inside a group that `knockout` says is a knockout
/// group — so the same spec can be asked in the position that refuses it and in the one
/// that does not.
///
/// The spec comes from a closure rather than a value because one case allocates a soft
/// mask, and a `MaskId` is scene-scoped: a mask defined in another builder is
/// [`SceneError::UnknownMask`] here, which would have made that case test the wrong
/// refusal.
fn built(
    knockout: bool,
    nested: impl FnOnce(&mut SceneBuilder) -> GroupSpec,
) -> Option<SceneError> {
    let mut builder = SceneBuilder::new();
    let nested = nested(&mut builder);
    builder
        .group(group(knockout, 1.0, Compose::SrcOver), |outer| {
            cover(outer)?;
            outer.group(nested, |body| {
                body.rect(full(), Affine::IDENTITY, OBJECT, None, None)
            })
        })
        .err()
}

/// The five shapes of ordinary nested group this refusal is about, each as a closure over
/// the builder that will hold it.
type Nested = fn(&mut SceneBuilder) -> GroupSpec;

fn plain(_: &mut SceneBuilder) -> GroupSpec {
    group(false, 1.0, Compose::SrcOver)
}

fn with_alpha(_: &mut SceneBuilder) -> GroupSpec {
    group(false, NESTED_ALPHA, Compose::SrcOver)
}

fn with_mask(builder: &mut SceneBuilder) -> GroupSpec {
    let mask = builder
        .mask(MaskKind::Alpha, None, |body| {
            body.rect(
                full(),
                Affine::IDENTITY,
                Color::new(1.0, 1.0, 1.0, 0.5),
                None,
                None,
            )
        })
        .unwrap();
    GroupSpec {
        mask: Some(mask),
        ..group(false, 1.0, Compose::SrcOver)
    }
}

fn with_blend(_: &mut SceneBuilder) -> GroupSpec {
    GroupSpec {
        blend: BlendMode::Multiply,
        ..group(false, NESTED_ALPHA, Compose::SrcOver)
    }
}

fn knocking_out(_: &mut SceneBuilder) -> GroupSpec {
    group(true, NESTED_ALPHA, Compose::SrcOver)
}

const CASES: [(&str, Nested); 5] = [
    ("a plain isolated group", plain),
    ("a constant alpha of its own", with_alpha),
    ("a soft mask of its own", with_mask),
    ("a blend mode of its own", with_blend),
    ("a knockout group inside a knockout group", knocking_out),
];

/// **The defect this file was written for.** Every shape of ordinary group, as an element
/// of a knockout group, is refused by name rather than composited by §11.3.6.
///
/// Five constructions, and each was measured drawing the wrong page before the refusal
/// existed (`doc/notes-nested-knockout.md` §2): a plain isolated group, one carrying a
/// constant alpha, one carrying a soft mask, one carrying a blend mode, and a knockout
/// group nested in a knockout group. None of the five is a special case of another — they
/// reach the wrong answer through different fields — so all five are named here rather
/// than one standing for the rest.
#[test]
fn a_group_that_is_an_element_of_a_knockout_group_is_refused() {
    for (what, nested) in CASES {
        assert_eq!(
            built(true, nested),
            Some(SceneError::KnockoutElementGroupUnsupported),
            "{what} is an element of a knockout group, and its shape is not its alpha"
        );
        // **The control the refusal needs**: the same spec, one flag away. If a case were
        // refused for something other than its position — an unknown mask identifier, an
        // alpha out of range — it would be refused here too, and the assertion above
        // would be reading the wrong variant.
        assert_eq!(
            built(false, nested),
            None,
            "{what} is an ordinary element of an ordinary group, which §11.3.6 composites"
        );
    }

    // And the builder survives a refusal: the group is discarded whole.
    let mut after = SceneBuilder::new();
    let _ = after.group(group(true, 1.0, Compose::SrcOver), |outer| {
        outer.group(group(false, 1.0, Compose::SrcOver), |_| Ok(()))
    });
    cover(&mut after).expect("the builder survives a refusal");
    assert_eq!(after.finish().cost().commands, 1);
}

/// **The control the refusal needs, and the reason its predicate is not the transitive
/// one.** §11.4.6's two stages are accepted inside a knockout group — and so are the
/// groups *inside* each half.
///
/// `pdf-model`'s `stated_shape` maps a group's shape to the group of its elements' shapes,
/// so a shape half that stands for a group holds groups. A refusal keyed on "anywhere
/// below a knockout group" would refuse the caller's own expansion of the clause this
/// refusal is about; one keyed on "an element of a knockout group" does not, because a
/// group inside an ordinary group is composited by §11.3.6 whatever encloses it.
#[test]
fn the_clauses_own_two_stages_are_not_refused_at_any_depth() {
    let mut builder = SceneBuilder::new();
    builder
        .group(group(true, 1.0, Compose::SrcOver), |outer| {
            cover(outer)?;
            // Each half's body is itself a group holding a group — three levels below the
            // knockout group, all of them ordinary composites.
            for compose in [Compose::DestOut, Compose::Plus] {
                outer.group(group(false, 1.0, compose), |half| {
                    half.group(group(false, 1.0, Compose::SrcOver), |inner| {
                        inner.group(group(false, 1.0, Compose::SrcOver), |deepest| {
                            deepest.rect(full(), Affine::IDENTITY, OBJECT, None, None)
                        })
                    })
                })?;
            }
            Ok(())
        })
        .expect("§11.4.6's two stages state the shape, and their bodies are ordinary");

    // A group inside an ordinary group is untouched wherever that ordinary group sits,
    // including as a staged half's body above; here it is at the root, for the plain case.
    let mut at_the_root = SceneBuilder::new();
    at_the_root
        .group(group(false, 1.0, Compose::SrcOver), |outer| {
            outer.group(group(false, NESTED_ALPHA, Compose::SrcOver), |inner| {
                inner.rect(full(), Affine::IDENTITY, OBJECT, None, None)
            })
        })
        .expect("an ordinary group's elements are §11.3.6's, at any depth");
}

// ------------------------------------------------------------------ the difference

/// The four rasters §11.4.6's line is written over, plus the two frames under test.
struct Measured {
    /// `(worst premultiplied deviation, partially covered pixels)` for the staged pair.
    staged: (f32, u32),
    /// The same for §11.3.6's composite of the same content — what the refused
    /// construction encodes to, byte for byte.
    ordinary: (f32, u32),
}

fn measure(device: &mut Device, outline: OutlineId) -> Measured {
    // P: what the outer knockout group holds before the element. A knockout group of one
    // rectangle holds no group element, so it is a scene the builder still accepts.
    let mut before = SceneBuilder::new();
    before
        .group(group(true, 1.0, Compose::SrcOver), cover)
        .unwrap();
    let before = render(device, &before.finish(), SIZE, SIZE);

    // f: §11.3.7.2's union of the shapes of the objects the nested group contains, which
    // for one opaque fill is the fill's own coverage. Read from the device, not assumed.
    let mut shape = SceneBuilder::new();
    wedge_fill(&mut shape, outline, Color::new(1.0, 1.0, 1.0, 1.0)).unwrap();
    let shape = render(device, &shape.finish(), SIZE, SIZE);

    // S: the nested group's own premultiplied deposit, drawn onto transparency.
    let mut deposit = SceneBuilder::new();
    deposit
        .group(group(false, NESTED_ALPHA, Compose::SrcOver), |body| {
            wedge_fill(body, outline, OBJECT)
        })
        .unwrap();
    let deposit = render(device, &deposit.finish(), SIZE, SIZE);

    // §11.4.6, stated: erase by the shape half — the same content drawn opaque, which is
    // the only way a group's shape reaches a raster — then deposit the object half.
    let mut staged = SceneBuilder::new();
    staged
        .group(group(true, 1.0, Compose::SrcOver), |outer| {
            cover(outer)?;
            outer.group(group(false, 1.0, Compose::DestOut), |half| {
                wedge_fill(half, outline, OBJECT)
            })?;
            outer.group(group(false, NESTED_ALPHA, Compose::Plus), |half| {
                wedge_fill(half, outline, OBJECT)
            })
        })
        .unwrap();
    let staged = render(device, &staged.finish(), SIZE, SIZE);

    // §11.3.6's composite of the same nested group over the same cover — the encode the
    // refused construction produces, reached through the position that still accepts it.
    let mut ordinary = SceneBuilder::new();
    ordinary
        .group(group(false, 1.0, Compose::SrcOver), |outer| {
            cover(outer)?;
            outer.group(group(false, NESTED_ALPHA, Compose::SrcOver), |body| {
                wedge_fill(body, outline, OBJECT)
            })
        })
        .unwrap();
    let ordinary = render(device, &ordinary.finish(), SIZE, SIZE);

    Measured {
        staged: deviation_from_the_clause(&before, &shape, &deposit, &staged),
        ordinary: deviation_from_the_clause(&before, &shape, &deposit, &ordinary),
    }
}

/// **The refusal is over a real difference.** The construction that is refused would have
/// drawn §11.3.6's composite, and that misses §11.4.6's line by two orders of magnitude
/// more than unorm rounding — while the construction that is *not* refused hits it.
///
/// A refusal whose two readings agreed would be a formality; this file measures both, as
/// `tests/mask_shape_or_opacity.rs` does, so the gate says what the caller is being spared
/// rather than only that a variant exists.
#[test]
fn the_refused_composite_misses_the_clause_that_the_stages_hit() {
    let mut device = device();
    let outline = wedge(&mut device);
    let Measured { staged, ordinary } = measure(&mut device, outline);
    eprintln!(
        "§11.4.6's line: staged {:.2} of 255, §11.3.6's composite {:.2} of 255",
        staged.0, ordinary.0
    );

    assert!(
        staged.1 > 30,
        "the nested group's shape must have partially covered pixels for this to mean \
         anything: {}",
        staged.1
    );
    assert_eq!(
        staged.1, ordinary.1,
        "both frames are measured against the same shape raster"
    );
    assert!(
        staged.0 <= 3.0,
        "§11.4.6's two stages on two groups are the clause's own line; worst \
         premultiplied deviation {}",
        staged.0
    );
    assert!(
        ordinary.0 >= 16.0,
        "and §11.3.6's composite of the same group must not be — a refusal whose two \
         readings agree spares the caller nothing: {}",
        ordinary.0
    );
}
