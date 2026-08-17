//! What an 8-bit raster does to a mark whose ink is under one of its levels.
//!
//! # Where the question comes from
//!
//! hayro #60, by way of the caller's `doc/HAYRO_ISSUES_FOR_QUORRA.md` §6. The reporter
//! asked whether colour is processed above 8 bits per channel and dithered on the way
//! down; the maintainer answered candidly that speed is the priority. The caller's version
//! of the question is the one this file answers, and it is sharper: *what an 8-bit raster
//! does to a mark whose ink is under one of its levels.*
//!
//! **ADR 0010 settled this, deliberately.** Layers, masks and the target are 8-bit
//! (`Rgba8Unorm`, `R8Unorm`), and the mask's exact byte agreement with the caller's
//! `SoftMask::value` is the precondition that made it the right trade. Nothing here
//! re-opens it. This file states what the decision *costs*, derives the boundary from the
//! arithmetic, and pins it — so that the cost is a number somebody can quote rather than a
//! property somebody would have to discover.
//!
//! # What the specification says, which is less than one might expect
//!
//! Clause 11 computes in real numbers and states no storage precision anywhere. What it
//! does state, twice and in NOTEs — non-normative both times — is that committing to a
//! raster loses information. §11.2:
//!
//! > The order in which objects are specified determines the stacking order but not
//! > necessarily the order in which the objects are actually painted onto the page. In
//! > particular, the transparency model does not require a PDF processor to rasterize
//! > objects immediately or to commit to a raster representation at any time before
//! > rendering the entire stack onto the page. This is important, since rasterization
//! > often causes significant loss of information and precision that is best avoided
//! > during intermediate stages of the transparency computation.
//!
//! and §11.7.2:
//!
//! > To minimise the accumulation of round off errors and avoid additional errors arising
//! > from the use of linear group colour spaces, more precision is needed for intermediate
//! > results than is typically used to represent either the original source data or the
//! > final rasterized results.
//!
//! So this is a place the specification advises rather than requires, we took the other
//! branch with a reason, and CLAUDE.md principle 5's rule for that case applies: say so
//! plainly and document the choice *as* a choice. These tests are that documentation with
//! a gate under it.
//!
//! # The arithmetic, and where the boundary actually is
//!
//! One mark of shape × opacity `a` and colour `s` over a destination `d`, composited
//! `SrcOver` (§11.3.6 with a Normal blend), stored to a UNORM8 attachment:
//!
//! ```text
//!   v    = d·(1 − a) + a·s          the clause's arithmetic, in reals
//!   byte = round(255 · v)           the store, round-to-nearest
//! ```
//!
//! Take the worst case a document offers: black ink on white paper, `s = 0`, `d = 1`. Then
//! `v = 1 − a` and `byte = 255 − round(255·a)`. **The stored byte moves if and only if
//! `round(255·a) ≥ 1`, that is `a ≥ 1/510`.**
//!
//! Two consequences, and the second is the one worth carrying:
//!
//! 1. The floor is **half** a level, not one. A mark carrying between half a level and a
//!    whole level of ink is rounded *up* to a whole level, so it is drawn heavier than its
//!    ink, not lighter. "Under one 255th" is the wrong phrase for what disappears; under
//!    one 510th is right.
//! 2. **The quantisation is per composite, not per frame.** Every mark reads an 8-bit
//!    destination and writes an 8-bit destination, so a mark under the floor leaves the
//!    byte exactly where it was — and the next one starts from the same place. A thousand
//!    of them are lost one at a time and never accumulate. That is the whole difference
//!    between this design and one that composites into a wider buffer, and it is what the
//!    §11.2 NOTE above is warning about.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects
)]

use quorra_gpu::Device;
use quorra_scene::{
    Affine, BlendMode, Color, Compose, GroupSpec, Point, Rect, Scene, SceneBuilder,
};

mod common;

use common::headless::{device, render};

/// A small square target; every assertion reads its centre, which no fixture's edge
/// coverage reaches.
const SIDE: u32 = 4;

/// One 8-bit level, as a fraction of the channel's full range.
const LEVEL: f32 = 1.0 / 255.0;

/// The floor derived in this file's header: a mark composited onto an opaque white
/// backdrop moves the stored byte exactly when its opacity reaches half a level.
const FLOOR: f32 = LEVEL / 2.0;

/// How many sub-level marks the accumulation tests stack. Two hundred marks at a quarter
/// of a level carry fifty levels of ink between them: enough that a renderer accumulating
/// in anything wider than eight bits would produce a plainly grey page.
const STACK: usize = 200;

/// The whole target, in device pixels.
fn page() -> Rect {
    Rect::new(Point::new(0.0, 0.0), Point::new(SIDE as f32, SIDE as f32))
}

/// Opaque white paper: the destination every derivation in this file assumes, and the one
/// that makes black ink's arithmetic `v = 1 − a` exactly.
fn paper(builder: &mut SceneBuilder) {
    builder
        .rect(
            page(),
            Affine::IDENTITY,
            Color::new(1.0, 1.0, 1.0, 1.0),
            None,
            None,
        )
        .expect("an opaque white page");
}

/// `count` full-page black marks at opacity `alpha`, over white paper.
fn ink_on_paper(count: usize, alpha: f32) -> Scene {
    let mut builder = SceneBuilder::new();
    paper(&mut builder);
    for _ in 0..count {
        builder
            .rect(
                page(),
                Affine::IDENTITY,
                Color::new(0.0, 0.0, 0.0, alpha),
                None,
                None,
            )
            .expect("§11.3.7.2's range holds every alpha this file uses");
    }
    builder.finish()
}

/// The red channel of the target's centre pixel.
fn centre(device: &mut Device, scene: &Scene) -> u8 {
    let pixels = render(device, scene, SIDE, SIDE);
    let at = (((SIDE / 2) * SIDE + SIDE / 2) * 4) as usize;
    assert_eq!(
        pixels[at + 3],
        255,
        "black over opaque white stays opaque (§11.3.6: αr = 1 whenever αb = 1), so the \
         straight-alpha readback is the stored value unchanged"
    );
    pixels[at]
}

/// **A quarter of a level of ink leaves the page byte-identical.**
///
/// `a = 0.25/255`, so `255·v = 254.75` and `round` gives 255: the paper is unchanged. The
/// margin from the rounding tie is a quarter of a level in the stored value, which is far
/// outside any adapter's freedom in a UNORM store.
#[test]
fn a_mark_of_a_quarter_level_leaves_the_paper_where_it_was() {
    let mut device = device();
    let blank = centre(&mut device, &ink_on_paper(0, 0.0));
    assert_eq!(
        blank, 255,
        "the paper is white before anything is drawn on it"
    );
    assert_eq!(
        centre(&mut device, &ink_on_paper(1, LEVEL / 4.0)),
        255,
        "a mark below the half-level floor is not drawn at all; that is the cost \
         ADR 0010 accepted and §11.2's NOTE describes"
    );
}

/// **Three quarters of a level of ink is drawn as a whole level.**
///
/// `a = 0.75/255`, so `255·v = 254.25` and `round` gives 254. The floor is half a level
/// rather than one, and below it nothing is drawn while above it a whole level is — the
/// raster is coarse here, not timid.
#[test]
fn a_mark_of_three_quarters_of_a_level_is_drawn_as_a_whole_one() {
    let mut device = device();
    assert_eq!(
        centre(&mut device, &ink_on_paper(1, LEVEL * 0.75)),
        254,
        "three quarters of a level of ink rounds up to one whole level"
    );
}

/// The boundary itself, from both sides, one level of stored value apart.
///
/// The two alphas straddle [`FLOOR`] by a quarter of a level each, which is the widest
/// margin the claim admits: closer to the tie and the assertion would be about a
/// driver's rounding mode rather than about this library's storage.
#[test]
fn the_floor_is_half_a_level_and_the_page_says_so_on_both_sides() {
    let mut device = device();
    let under = centre(&mut device, &ink_on_paper(1, FLOOR - LEVEL / 4.0));
    let over = centre(&mut device, &ink_on_paper(1, FLOOR + LEVEL / 4.0));
    assert_eq!(under, 255, "below half a level: nothing is drawn");
    assert_eq!(over, 254, "above half a level: one whole level is drawn");
}

/// **Two hundred marks under the floor are lost one at a time, and never add up.**
///
/// The question underneath hayro #60. Each mark reads and writes the same 8-bit
/// destination, so each one independently rounds to no change; fifty levels of ink go in
/// and the page is still paper. A renderer accumulating in f32 and quantising once would
/// draw them all.
#[test]
fn many_marks_under_the_floor_never_accumulate() {
    let mut device = device();
    assert_eq!(
        centre(&mut device, &ink_on_paper(STACK, LEVEL / 4.0)),
        255,
        "{STACK} marks at a quarter of a level each carry {} levels of ink and leave the \
         page byte-identical: the quantisation is per composite, not per frame",
        STACK / 4
    );
}

/// The control for the test above: the same stack, above the floor, does accumulate.
///
/// Without this the assertion "the page is still 255" is satisfied by a fixture that draws
/// nothing at all — the trap `tests/no_ink.rs` names and `doc/HANDOVER.md` states as a
/// rule. Four levels a mark over two hundred marks leaves the page nearly black.
#[test]
fn the_same_stack_above_the_floor_darkens_the_page() {
    let mut device = device();
    let inked = centre(&mut device, &ink_on_paper(STACK, LEVEL * 4.0));
    assert!(
        inked < 100,
        "the stack itself must be able to move the page, or the sub-level result above is \
         a fixture that draws nothing: {inked}"
    );
}

/// A transparency group does not rescue them either, because its layer is 8 bits too.
///
/// §11.2's NOTE says a processor need not commit to a raster before the whole stack is
/// rendered; ADR 0010 commits at every group boundary and every mark, and this is what
/// that costs. Stated here rather than inferred, because "the intermediate buffer has more
/// precision" is the first thing a reader assumes and it is not true.
#[test]
fn a_transparency_group_does_not_widen_the_accumulation() {
    let mut device = device();
    let mut builder = SceneBuilder::new();
    paper(&mut builder);
    builder
        .group(
            GroupSpec {
                alpha: 1.0,
                blend: BlendMode::Normal,
                clip: None,
                knockout: false,
                mask: None,
                isolated: false,
                compose: Compose::SrcOver,
            },
            |body| {
                for _ in 0..STACK {
                    body.rect(
                        page(),
                        Affine::IDENTITY,
                        Color::new(0.0, 0.0, 0.0, LEVEL / 4.0),
                        None,
                        None,
                    )?;
                }
                Ok(())
            },
        )
        .expect("a non-isolated group over an opaque backdrop (ADR 0019)");
    assert_eq!(
        centre(&mut device, &builder.finish()),
        255,
        "the group's own layer is Rgba8Unorm (ADR 0010), so its elements quantise into it \
         exactly as they would quantise into the page"
    );
}
