//! The caller's four layers, and how many fragments the present pass draws for them.
//!
//! **Exact, and adapter-free.** Everything in this file is arithmetic over placements —
//! ADR 0052's seam: a claim about *how many* is a count and is exact, a claim about
//! *how fast* is a duration and this machine cannot measure one. `rate` is the other
//! side of that seam and says so in its own comment.
//!
//! The layers are `PLAN.md`'s M9 entry — a page, a selection, a sidebar and a modal
//! card — sized from the caller's own tree in `doc/notes-present-quad.md` §1. They are
//! carried here at **their** window (2048 × 2560) purely so that
//! [`the_shapes_are_the_ones_adr_0058_counted`] can check this file's arithmetic against
//! ADR 0058's published totals; the run measures at whatever window the display can
//! actually hold, which is not that one.

use std::fmt::Write as _;

/// One layer of an arrangement: what size its texture is and where its placement puts
/// the texture's origin. Every placement in the caller's arrangement is a translation —
/// a reprojection is a scale and a translation, and their overlays are drawn in window
/// pixels — so a shape needs no linear part to be the arrangement they have.
#[derive(Clone, Copy)]
pub(crate) struct Shape {
    /// What this layer is, for the row it prints.
    pub(crate) name: &'static str,
    /// The layer texture's extent in texels.
    pub(crate) extent: (u32, u32),
    /// Where the placement puts texel (0, 0) on the window, in device pixels.
    pub(crate) at: (f32, f32),
}

/// The caller's four layers at the caller's own 2048 × 2560 — their §2's 1280 × 1600
/// window at a device scale of 1.6.
///
/// **The selection's extent was recovered rather than read.** `doc/notes-present-quad.md`
/// §1 records the other three from the caller's tree and leaves the selection at "see
/// below", because a selection has no natural size; what it does record is the totals its
/// instrument produced, and two of those rows (with the modal and without it) give the
/// same answer for the one unknown: a rectangle whose dilated area is 314 924 fragments.
/// Of that number's factorisations only a few fit inside the page, and a text selection
/// is wide and short, so 1200 × 260 is the one taken. **The orientation changes no count
/// here** — the rectangle is well inside the window either way — which is why recovering
/// it is legitimate and why the gate below is a check on this file rather than on the
/// notes.
pub(crate) const AT_THEIR_WINDOW: [Shape; 4] = [
    Shape {
        name: "page",
        extent: (1568, 2217),
        at: (480.0, 171.0),
    },
    Shape {
        name: "selection",
        extent: (1200, 260),
        at: (640.0, 900.0),
    },
    Shape {
        name: "sidebar",
        extent: (480, 2560),
        at: (0.0, 0.0),
    },
    Shape {
        name: "modal card",
        extent: (2048, 2560),
        at: (0.0, 0.0),
    },
];

/// The caller's window, which does not fit the display this is measured on.
pub(crate) const THEIR_WINDOW: (u32, u32) = (2048, 2560);

/// The device scale their window is at, and therefore the divisor between their layer
/// sizes and the ones a 2880 × 1800 display can hold.
pub(crate) const THEIR_SCALE: f64 = 1.6;

/// The same four layers with every overlay left at the **window's** size, which is the
/// arrangement the caller has today: `viewer-ui`'s `highlight_list` and `Notice::draw`
/// each build a display list at the window's size, because their present composes
/// everything into one scene. Ported unchanged into layer textures, every overlay is a
/// window-sized texture — and ADR 0058 buys 8.4 % rather than 51 %.
///
/// The page keeps its own extent and placement: it is the one layer that is not drawn in
/// window pixels in either arrangement.
pub(crate) fn window_sized(content: &[Shape], window: (u32, u32)) -> Vec<Shape> {
    content
        .iter()
        .map(|shape| {
            if shape.name == "page" {
                *shape
            } else {
                Shape {
                    name: shape.name,
                    extent: window,
                    at: (0.0, 0.0),
                }
            }
        })
        .collect()
}

/// The caller's arrangement divided down to a window a 2880 × 1800 display can hold.
///
/// Every extent is theirs divided by [`THEIR_SCALE`] and rounded, and every placement is
/// theirs divided by the same number and **not** rounded — a placement is an `f32` in the
/// API and rounding it would move the layer for no reason. The window this is asked for
/// is the one the window system actually gave us, so a window manager that refused the
/// size we asked for produces honest counts for the size we got rather than pretty ones
/// for the size we wanted.
pub(crate) fn scaled_to(window: (u32, u32)) -> Vec<Shape> {
    let scale = f64::from(window.0) / f64::from(THEIR_WINDOW.0);
    AT_THEIR_WINDOW
        .iter()
        .map(|shape| Shape {
            name: shape.name,
            extent: (
                scaled_extent(shape.extent.0, scale),
                scaled_extent(shape.extent.1, scale),
            ),
            at: (
                scaled_place(shape.at.0, scale),
                scaled_place(shape.at.1, scale),
            ),
        })
        .collect()
}

/// A texel extent scaled and rounded, never to zero: `wgpu` will not make a texture with
/// an empty extent, so a scale that rounds a thin layer away has to stop at one texel
/// rather than produce a descriptor the device refuses.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn scaled_extent(texels: u32, scale: f64) -> u32 {
    ((f64::from(texels) * scale).round() as u32).max(1)
}

#[allow(clippy::cast_possible_truncation)]
fn scaled_place(at: f32, scale: f64) -> f32 {
    (f64::from(at) * scale) as f32
}

/// The fragments `present.wgsl`'s vertex stage produces for one layer: the layer's own
/// rectangle, grown outward one whole pixel each side and clamped to the target
/// (ADR 0058, and `Layer::device_bounds` is the implementation this models).
///
/// **A model of the library rather than the library**, on purpose: the count wanted here
/// is one a reader can check against ADR 0058's table with a calculator, and the five
/// `device_bounds` unit tests are what hold the library to it. The two agree because
/// [`the_shapes_are_the_ones_adr_0058_counted`] says the totals do.
pub(crate) fn rectangle_fragments(shape: Shape, target: (u32, u32)) -> u64 {
    let span = |origin: f32, texels: u32, limit: u32| -> u64 {
        let limit = f64::from(limit);
        let low = (f64::from(origin).floor() - 1.0).clamp(0.0, limit);
        let high = (f64::from(origin) + f64::from(texels)).ceil() + 1.0;
        // Both ends are inside `[0, limit]` after the clamp, so the difference is a
        // whole number this size and the conversion cannot lose anything.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            (high.clamp(0.0, limit) - low) as u64
        }
    };
    span(shape.at.0, shape.extent.0, target.0) * span(shape.at.1, shape.extent.1, target.1)
}

/// What the whole arrangement costs the pass, and what it would have cost before
/// ADR 0058 — one full-screen triangle per layer, whatever each layer's size.
pub(crate) fn cost(shapes: &[Shape], target: (u32, u32)) -> (u64, u64) {
    let rectangle = shapes
        .iter()
        .map(|shape| rectangle_fragments(*shape, target))
        .sum();
    let triangle = shapes.len() as u64 * u64::from(target.0) * u64::from(target.1);
    (triangle, rectangle)
}

/// One arrangement's row, for the report.
pub(crate) fn row(label: &str, shapes: &[Shape], target: (u32, u32)) -> String {
    let (triangle, rectangle) = cost(shapes, target);
    let mut line = format!(
        "  {label:<34} {} layers  triangle {triangle:>11}  rectangle {rectangle:>11}  \
         {:>5.1} %",
        shapes.len(),
        100.0 * rectangle as f64 / triangle as f64,
    );
    for shape in shapes {
        let _ = write!(
            line,
            "\n      {:<12} {:>4} x {:<4} at ({:>7.2}, {:>7.2})  {:>10} fragments",
            shape.name,
            shape.extent.0,
            shape.extent.1,
            shape.at.0,
            shape.at.1,
            rectangle_fragments(*shape, target),
        );
    }
    line
}

/// **The gate on this file**: the arithmetic above reproduces ADR 0058's published
/// totals at the caller's own window, for the three rows whose layer extents that ADR
/// records.
///
/// It needs no window, no device and no adapter, which is why it runs first and runs
/// under `--check`. What it catches is the thing an arrangement in an example rots into:
/// a shape edited for a run and never checked back against the decision it was taken
/// from. Verified able to fail by moving one extent by a texel — the row names itself and
/// both numbers.
pub(crate) fn the_shapes_are_the_ones_adr_0058_counted() {
    let content = &AT_THEIR_WINDOW[..];
    let same = |label: &str, shapes: &[Shape], want: u64| {
        let (_, got) = cost(shapes, THEIR_WINDOW);
        let (width, height) = THEIR_WINDOW;
        assert_eq!(
            got, want,
            "{label}: this file's shapes draw {got} fragments at {width} x {height}; \
             ADR 0058 and doc/notes-present-quad.md §2 count {want}",
        );
    };
    same(
        "window-sized overlays",
        &window_sized(content, THEIR_WINDOW),
        19_210_251,
    );
    same("content-sized overlays", content, 10_270_775);
    same("content-sized overlays, no modal", &content[..3], 5_027_895);
}
