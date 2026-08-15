//! A page whose glyph tiles overflow the atlas replays like any other page (ADR 0050).
//!
//! One shape of frame used to invalidate its own encode on every frame: the tile that
//! fell through to the scratch sheet made the device repack the atlas, the repack bumped
//! the generation the encode had just been stored under, and the next frame did it again.
//! Twelve frames, twelve encodes, on a page a reader is holding still.
//!
//! Both tests here live in the narrow band that produced it — a working set that fits the
//! atlas **by bytes** and does not fit it **by packing** — because that is ADR 0024's
//! repack condition and nothing outside the band reproduces the loop. The two guard
//! assertions in [`assert_overflows_but_fits_by_bytes`] check that the fixture is still
//! inside it: a page that never overflows replays trivially, so a fixture that drifted out
//! would go on passing while testing nothing.
//!
//! `Counters::atlas_repacked` is the observable that separates settling from thrashing,
//! and it is why these are properties rather than clocks.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Counters, EncodeSource, Options, RetainedScene, Target};

mod common;

use common::headless::pixels;
use common::retained::{device_with, retained_frame, text_page, viewport};

/// The atlas and the page that overflows it, sized so that **the working set fits by
/// bytes and does not fit by packing** — which is the one band in which the old repack
/// rule fired forever.
///
/// A 96×96 atlas holds 9 216 texels and every tile here is one shape at one integer
/// phase — 14×18, or 252 texels — so the packer's capacity is arithmetic rather than
/// luck: six tiles to a shelf (96/14, and 12 texels wasted), five shelves (96/18, and
/// 6 rows wasted), **30 tiles**. The 34 distinct shapes ask for 8 568 texels, which is
/// comfortably inside 9 216 and four tiles more than the shelves hold.
///
/// That gap between the two arithmetics is the whole of the band: the byte test says a
/// repack would fit the working set and the packer says it would not, every frame,
/// forever. The two guard assertions below check the condition rather than trusting
/// these numbers to keep meaning what they mean — the tile size is the rasteriser's
/// answer, not this file's.
const OVERFLOW_ATLAS: u64 = 96 * 96;
const OVERFLOW_DISTINCT: u32 = 34;
const OVERFLOW_PLACEMENTS: u32 = 68;
const OVERFLOW_SIDE: f32 = 13.0;

/// The two conditions that make a frame the one this file is about: a tile went to the
/// scratch sheet, and the frame's own working set would fit the atlas by bytes.
///
/// Asserted at the top of each test, because a fixture that silently stops reproducing
/// its condition is a test that proves nothing — and here it would go on passing, since
/// a page that never overflows replays trivially.
fn assert_overflows_but_fits_by_bytes(counters: &Counters, why: &str) {
    assert!(
        counters.tiles > 0,
        "{why}: this fixture only tests what it claims to if the atlas refused a tile: {counters:?}"
    );
    assert!(
        counters.atlas_working_set_bytes <= OVERFLOW_ATLAS,
        "{why}: and only if the working set fits the atlas by bytes, which is what makes \
         a repack look worth taking: {counters:?}"
    );
}

/// **The headline of ADR 0050.** A page whose glyph tiles overflow the atlas replays
/// like any other page, and draws the pixels a fresh encode draws.
///
/// It did not, and the reason was a loop the frame closed on itself: the tile that fell
/// through to the scratch sheet made the device repack the atlas, the repack bumped the
/// generation the encode had just been stored under, and the next frame re-encoded and
/// did the same again. Twelve frames, twelve encodes, on a page a reader is holding
/// still. Magnified text at a modest atlas budget is that shape.
///
/// Nothing foreign is in this atlas — it is a fresh device — so a repack would re-pack
/// the frame's own tiles in the frame's own encounter order and reproduce the layout it
/// replaced, tile for tile, including the one that overflows. That is why not taking it
/// is not a compromise, and `atlas_repacked` says it happened zero times.
#[test]
fn a_page_the_atlas_cannot_hold_replays_after_its_first_frame() {
    let mut device = device_with(&Options {
        atlas_budget: OVERFLOW_ATLAS,
        ..Options::default()
    });
    let (scene, _) = text_page(
        &mut device,
        OVERFLOW_PLACEMENTS,
        OVERFLOW_DISTINCT,
        OVERFLOW_SIDE,
    );
    let viewport = viewport();
    let mut retained = RetainedScene::new(scene.clone());

    let first = retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "the first frame of a handle has nothing to replay",
    );
    assert_overflows_but_fits_by_bytes(&first.counters(), "the first frame");
    assert!(
        !first.counters().atlas_repacked,
        "an atlas holding nothing but this frame's own tiles repacks to the layout it \
         already has: {:?}",
        first.counters()
    );
    let first = pixels(first);

    for round in 0..4 {
        let frame = retained_frame(
            &mut device,
            &mut retained,
            &viewport,
            EncodeSource::Replayed,
            "an overflowing page is still an unchanged page",
        );
        assert!(
            !frame.counters().atlas_repacked,
            "round {round}: a replay inserts nothing and so settles nothing: {:?}",
            frame.counters()
        );
        assert_eq!(
            pixels(frame),
            first,
            "round {round}: the replayed page must be the page that was encoded"
        );
    }

    // The other direction, and the one that would catch a replay drawing from a moved
    // atlas: a frame encoded from scratch *now*, against the atlas as it stands.
    let fresh = device
        .render(&scene, &viewport, Target::Readback)
        .expect("the page draws");
    assert_eq!(
        fresh.encode_source(),
        EncodeSource::Encoded,
        "`render` retains nothing and always encodes"
    );
    assert!(
        !fresh.counters().atlas_repacked,
        "and it finds nothing foreign to reclaim either: {:?}",
        fresh.counters()
    );
    assert_eq!(
        pixels(fresh),
        first,
        "a freshly encoded frame and a replayed one are the same page, to the byte"
    );
}

/// **The oscillation counter.** An atlas holding another page's tiles repacks **once**,
/// and the frame after it finds nothing left to reclaim.
///
/// This is the sequence the old rule could not end: repack, re-encode, overflow, repack.
/// `Counters::atlas_repacked` is what makes the difference between the two behaviours
/// observable from outside — a page that settles reports it true on one frame and false
/// on every later one, and a page that thrashes would report it true forever.
///
/// Two encodes and then replays for ever is the *most* a page can cost here, and that
/// bound is the decision: the first frame pays for the atlas it inherited, the second
/// pays for the layout that replaced it, and the third onwards pay nothing.
#[test]
fn an_atlas_holding_another_page_repacks_once_and_then_settles() {
    let mut device = device_with(&Options {
        atlas_budget: OVERFLOW_ATLAS,
        ..Options::default()
    });
    // A different page first, so the atlas holds tiles the page below will never ask
    // for. Its outlines are its own, so no key of it can collide with one of theirs.
    let (foreign, _) = text_page(&mut device, 6, 3, OVERFLOW_SIDE);
    let (crowded, _) = text_page(
        &mut device,
        OVERFLOW_PLACEMENTS,
        OVERFLOW_DISTINCT,
        OVERFLOW_SIDE,
    );
    let viewport = viewport();
    device
        .render(&foreign, &viewport, Target::Readback)
        .expect("the small page draws");

    let mut retained = RetainedScene::new(crowded);
    let first = retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "the first frame of a handle has nothing to replay",
    );
    assert_overflows_but_fits_by_bytes(&first.counters(), "the first frame");
    assert!(
        first.counters().atlas_repacked,
        "the atlas held another page's tiles and this frame had no room: reclaiming them \
         is the one thing a repack does: {:?}",
        first.counters()
    );

    let second = retained_frame(
        &mut device,
        &mut retained,
        &viewport,
        EncodeSource::Encoded,
        "the repack moved every texel origin the first encode named",
    );
    assert!(
        !second.counters().atlas_repacked,
        "and now there is nothing foreign left, so a second repack would reproduce this \
         very layout: {:?}",
        second.counters()
    );

    let mut repacks = 0_u32;
    for round in 0..6 {
        let frame = retained_frame(
            &mut device,
            &mut retained,
            &viewport,
            EncodeSource::Replayed,
            "the atlas has settled, so the encode made against it still holds",
        );
        repacks += u32::from(frame.counters().atlas_repacked);
        assert_eq!(repacks, 0, "round {round}: the atlas repacked again");
    }
}
