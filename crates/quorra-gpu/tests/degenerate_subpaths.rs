//! What a stroke deposits where its subpath has no length: ISO 32000-2 §8.5.3.2.
//!
//! The question comes from the caller's reading of hayro #296, where every glyph outline
//! came back beginning with a spurious `MoveTo((0, 0))` before its real one. A leading
//! degenerate `MoveTo` is invisible to a fill and is *not* obviously invisible to a
//! stroke, because §8.4.3.3 applies caps "at both ends of open subpaths" and a subpath of
//! one point is an open subpath. So: does this tree deposit a dot at the origin?
//!
//! §8.5.3.2's last paragraph answers all four shapes a producer can hand us, and the four
//! answers are not the same:
//!
//! > If a subpath is degenerate (consists of a single-point closed path or of two or more
//! > points at the same coordinates), the S operator shall paint it only if round line
//! > caps have been specified, producing a filled circle centred at the single point. If
//! > butt or projecting square line caps have been specified, S shall produce no output,
//! > because the orientation of the caps would be indeterminate. This rule shall apply
//! > only to zero-length subpaths of the path being stroked, and not to zero-length dashes
//! > in a dash pattern of a non-degenerate subpath. In the latter case, the line caps
//! > shall always be painted, since their orientation is determined by the direction of
//! > the underlying path except in the case of a degenerate subpath. A single-point open
//! > subpath (specified by a trailing m operator) shall produce no output.
//!
//! So the clause's answer to hayro #296 is the sentence at the end and it is unconditional:
//! a bare `MoveTo` **produces no output under every cap style**, and this file's first two
//! tests hold this tree to it. The disc is owed only to the *closed* single point and to
//! two-or-more coincident points, and only under round caps.
//!
//! # What actually reaches us, and why the last two tests still exist
//!
//! `RENDER_LIBRARY.md` §4.5 settles degenerate subpaths upstream — "we pre-split them;
//! draw what you are given" — and the caller's `pdf-render::degenerate::split_degenerate`
//! is that split: it strips every degenerate subpath out of the stroked path and emits
//! §8.5.3.2's circle as a **filled** outline under round caps, so what arrives here is a
//! stroke whose every subpath has length plus, sometimes, a fill of a circle. A degenerate
//! subpath is therefore not expected in a `Command::Stroke` at all.
//!
//! Nothing stops another producer sending one, so these tests build the outline by hand
//! through `SceneBuilder` and state what we do with each shape. Two of the four answers
//! agree with §8.5.3.2 directly; the other two — a single-point *closed* path and two
//! coincident points, both of which the clause makes a disc under round caps — draw
//! nothing here, and that is the inherited §4.5 divergence written down rather than
//! assumed. It is written down because it is exactly the shape of trap 2: if this side
//! ever assumed the caller stopped splitting, or that side ever assumed we grew the disc,
//! a round-cap dot would vanish with no test able to see it.

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

mod common;

use common::headless::{device, render};
use quorra_gpu::Device;
use quorra_scene::{
    Affine, BlendMode, Color, LineCap, LineJoin, Paint, Point, SceneBuilder, Segment, Stroke,
};

/// 64 pixels wide: 64 × 4 bytes = 256, the buffer-copy row alignment.
const SIZE: u32 = 64;

/// Half the stroke width. A disc of this radius at [`DOT`] is 113 pixels of ink on a
/// 4 096-pixel target, so a deposited cap cannot hide inside a rounding tolerance.
const HALF_WIDTH: f32 = 6.0;

/// Where the degenerate subpath sits: well inside the target and far from [`LINE`], so a
/// cap deposited there lands on pixels no other mark reaches.
const DOT: Point = Point::new(16.0, 16.0);

/// The real subpath the spurious `MoveTo` is appended to or prefixed with. Horizontal,
/// so its own butt caps are two clean vertical edges.
const LINE: (Point, Point) = (Point::new(12.0, 48.0), Point::new(52.0, 48.0));

/// The three cap styles of §8.4.3.3 Table 53, in the clause's own order.
const CAPS: [LineCap; 3] = [LineCap::Butt, LineCap::Round, LineCap::Square];

fn stroke_of(cap: LineCap) -> Stroke {
    Stroke {
        width: HALF_WIDTH * 2.0,
        adjust: false,
        cap,
        join: LineJoin::Miter,
        miter_limit: 10.0,
    }
}

/// Upload `segments`, stroke them under `cap` in opaque green on transparency, and hand
/// back the target's straight-alpha RGBA bytes.
fn stroked(device: &mut Device, segments: &[Segment], cap: LineCap) -> Vec<u8> {
    let outline = device.upload_outline(segments).expect("upload");
    let mut builder = SceneBuilder::new();
    builder
        .stroke(
            outline,
            Affine::IDENTITY,
            stroke_of(cap),
            Paint::Solid(Color::new(0.0, 0.8, 0.2, 1.0)),
            None,
            BlendMode::Normal,
            None,
        )
        .expect("valid stroke");
    let scene = builder.finish();
    render(device, &scene, SIZE, SIZE)
}

/// The real subpath alone: `MoveTo`, `LineTo`.
fn line() -> Vec<Segment> {
    vec![Segment::MoveTo(LINE.0), Segment::LineTo(LINE.1)]
}

/// The bytes of a target nothing drew on. `Target::Readback` hands back straight-alpha
/// RGBA (§3 of the brief), and a page renders onto transparency (§11.4.7), so this is
/// what "no output" looks like.
fn blank() -> Vec<u8> {
    vec![0_u8; (SIZE * SIZE * 4) as usize]
}

/// hayro #296's shape: a spurious `MoveTo` before the real one.
///
/// §8.5.3.2: "A single-point open subpath (specified by a trailing m operator) shall
/// produce no output." The clause's subject is the subpath, not its position in the path,
/// so the answer does not depend on the degenerate `MoveTo` coming first or last — and it
/// does not depend on the cap, because that sentence states no cap condition where the two
/// sentences above it do.
///
/// Byte identity against the same stroke without the spurious `MoveTo` is the assertion
/// rather than "no ink at `DOT`": a cap could also perturb the line's own coverage through
/// the winding rule, and equality catches that where a probe at one point would not.
#[test]
fn a_leading_degenerate_move_to_deposits_no_cap_under_any_style() {
    let mut device = device();
    for cap in CAPS {
        let mut with_stray = vec![Segment::MoveTo(DOT)];
        with_stray.extend(line());
        let stray = stroked(&mut device, &with_stray, cap);
        let plain = stroked(&mut device, &line(), cap);
        assert_eq!(
            stray, plain,
            "§8.5.3.2: a single-point open subpath produces no output, under {cap:?}"
        );
    }
}

/// The same sentence read the way it is written — "a trailing m operator" — which is the
/// shape hayro #296 is *not*, and it must answer the same.
#[test]
fn a_trailing_degenerate_move_to_deposits_no_cap_under_any_style() {
    let mut device = device();
    for cap in CAPS {
        let mut with_stray = line();
        with_stray.push(Segment::MoveTo(DOT));
        let stray = stroked(&mut device, &with_stray, cap);
        let plain = stroked(&mut device, &line(), cap);
        assert_eq!(
            stray, plain,
            "§8.5.3.2: a single-point open subpath produces no output, under {cap:?}"
        );
    }
}

/// A path that is nothing but a bare `MoveTo`: the whole scene draws nothing, whatever the
/// cap. §8.5.3.2's last sentence again, with no other mark in the frame to hide behind.
///
/// It is a `Command::Stroke` that reaches the encoder and is not refused — the outline is
/// valid (`ResourceStore::upload_outline` requires a leading `MoveTo` and this is one) and
/// the stroke is valid (a positive width). What it is not is a mark: `raster::flatten`
/// keeps no subpath of fewer than two points, so the expansion never sees an end to cap.
#[test]
fn a_path_that_is_only_a_move_to_draws_nothing_under_any_style() {
    let mut device = device();
    for cap in CAPS {
        let pixels = stroked(&mut device, &[Segment::MoveTo(DOT)], cap);
        assert_eq!(
            pixels,
            blank(),
            "§8.5.3.2: a single-point open subpath produces no output, under {cap:?}"
        );
    }
}

/// **The §4.5 divergence, stated so that neither side can assume the other took it.**
///
/// §8.5.3.2 gives a single-point *closed* path a disc under round caps: "the S operator
/// shall paint it only if round line caps have been specified, producing a filled circle
/// centred at the single point". This tree paints nothing — `raster::flatten` drops a
/// one-point subpath at `Segment::Close` exactly as it drops one at `MoveTo`.
///
/// That is correct for butt and square by the clause's own next sentence, and for round it
/// is the brief's §4.5 in force: the caller splits the degenerate subpath out and emits the
/// circle as a **fill** of its own geometry (`pdf-render::degenerate::split_degenerate`,
/// whose `dots` field is documented as "the circles §8.5.3.2 asks for, to be **filled**
/// with the stroking paint"), precisely so that neither rasteriser's round cap decides a
/// clause. So the disc reaching us is a `Command::Fill`, and a `Command::Stroke` of a
/// degenerate subpath is a shape this contract says cannot arrive.
///
/// This test pins what happens if one arrives anyway. Its expected value is *not* derived
/// from §8.5.3.2 — under round caps the clause asks for the disc this asserts is absent —
/// it is derived from §4.5's allocation of the decision, and it exists so that a change on
/// either side of the boundary fails a test on this one.
#[test]
fn a_single_point_closed_path_draws_nothing_here_because_4_5_places_the_disc_upstream() {
    let mut device = device();
    for cap in CAPS {
        let pixels = stroked(&mut device, &[Segment::MoveTo(DOT), Segment::Close], cap);
        assert_eq!(
            pixels,
            blank(),
            "§4.5 places §8.5.3.2's disc upstream, so nothing is drawn here under {cap:?}"
        );
    }
}

/// The clause's other degenerate shape — "two or more points at the same coordinates" —
/// which arrives as a `LineTo` back to the point the subpath started at rather than as a
/// `Close`. Same answer here, and the same §4.5 reason as the test above.
///
/// This one reaches a second guard as well as the first: the subpath survives
/// `raster::flatten` with two points, and `raster::stroke_polylines` then collapses the
/// coincident pair and declines the lone point. Both must hold for the answer to be
/// stable, which is why the shape is tested rather than inferred from the other.
#[test]
fn two_coincident_points_draw_nothing_here_because_4_5_places_the_disc_upstream() {
    let mut device = device();
    for cap in CAPS {
        let pixels = stroked(
            &mut device,
            &[Segment::MoveTo(DOT), Segment::LineTo(DOT)],
            cap,
        );
        assert_eq!(
            pixels,
            blank(),
            "§4.5 places §8.5.3.2's disc upstream, so nothing is drawn here under {cap:?}"
        );
    }
}

/// The control that makes the four tests above measurements rather than absences: at this
/// width and this position a round cap **is** visible, and a square cap is visible where a
/// butt cap is not. If a cap were deposited at `DOT` these numbers say by how much it
/// would be seen.
///
/// §8.4.3.3 Table 53: butt "shall be squared off at the endpoint … no projection beyond
/// the end of the path"; projecting square "shall continue beyond the endpoint … for a
/// distance equal to half the line width"; round is "a semicircular arc with a diameter
/// equal to the line width … around the endpoint". So over the line's two ends square adds
/// exactly `2 × half_width × width` of ink to butt's, and round adds a full disc of radius
/// `half_width`.
#[test]
fn the_caps_this_file_asserts_are_absent_are_ones_the_target_can_see() {
    let mut device = device();
    let mut ink = |cap: LineCap| -> f64 {
        let pixels = stroked(&mut device, &line(), cap);
        pixels
            .chunks_exact(4)
            .map(|p| f64::from(p[3]) / 255.0)
            .sum()
    };
    let butt = ink(LineCap::Butt);
    let square = ink(LineCap::Square);
    let round = ink(LineCap::Round);

    let half = f64::from(HALF_WIDTH);
    let squares = 2.0 * half * (2.0 * half);
    let disc = std::f64::consts::PI * half * half;
    // One quantisation step per pixel of a mark's boundary; the tolerance is generous
    // because this test is about the caps being *large*, not about their exact area —
    // `raster.rs`'s `each_cap_deposits_the_area_table_53_gives_it` holds the area itself.
    assert!(
        (square - butt - squares).abs() < 4.0,
        "§8.4.3.3: a projecting square cap adds half a width at each end ({square} − {butt} ≉ {squares})"
    );
    assert!(
        (round - butt - disc).abs() < 4.0,
        "§8.4.3.3: two round caps are one disc of the half width ({round} − {butt} ≉ {disc})"
    );
    assert!(
        disc > 100.0,
        "a cap deposited at the stray point would be {disc} pixels of ink, not a rounding"
    );
}
