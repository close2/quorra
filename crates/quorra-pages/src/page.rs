//! The named pages, and what each is recorded to cost.
//!
//! Seven of them are `doc/corpus-profile.md`'s archetypes and are gated as a set by
//! `crates/quorra-gpu/tests/archetypes.rs`. The two after them are pages an instrument
//! draws that are **not** archetypes, and they are named here rather than left inline so
//! that the difference is visible: each of them was, until 2026-08-17, a copy inside an
//! example whose comment said it was one of the seven.

use crate::archetype::{Archetype, Recorded};

/// A page no gate has priced. Every instrument-only page carries this; a page with a
/// row is a page `tests/archetypes.rs` compares by equality.
const UNPRICED: Option<Recorded> = None;

/// A recorded row, written positionally once so that ten `Some(Recorded { … })`
/// literals do not bury the numbers they exist to carry.
///
/// `(commands, culled, distinct outlines, atlas keys, clip regions, tiles, layer
/// textures, residue regions, residue tiles, coverage texels)` — the order
/// `tests/archetypes.rs` prints them in.
#[allow(clippy::too_many_arguments)] // ten counters, and a struct literal per page is worse
const fn row(
    commands: u64,
    commands_culled: u64,
    distinct_outlines: u64,
    atlas_distinct_keys: u64,
    clip_distinct_regions: u64,
    tiles: u64,
    layer_textures: u64,
    clip_residue_regions: u64,
    clip_residue_tiles: u64,
    coverage_texels: u64,
) -> Recorded {
    Recorded {
        commands,
        commands_culled,
        distinct_outlines,
        atlas_distinct_keys,
        clip_distinct_regions,
        tiles,
        layer_textures,
        clip_residue_regions,
        clip_residue_tiles,
        coverage_texels,
    }
}

/// The brief's window scale (§6.2), which is what the archetypes' counts were taken at.
const WIDTH: u32 = 1191;
/// The brief's window scale (§6.2).
const HEIGHT: u32 = 1684;

/// The fields every archetype shares, so that each page below states only what
/// distinguishes it.
const BLANK: Archetype = Archetype {
    name: "",
    width: WIDTH,
    height: HEIGHT,
    commands: 0,
    distinct: 0,
    segments: 0,
    side: 0.0,
    strokes: 0,
    images: 0,
    image_side: 0,
    clips: 0,
    clipped: 0,
    rect_clips: false,
    groups: 0,
    blended_groups: 0,
    recorded: UNPRICED,
};

/// The median corpus page: twelve commands, nine outlines. Most of a corpus is this,
/// and what it measures is the per-frame floor rather than any lane.
///
/// **Recorded row.** Twelve fills over nine outlines at twelve distinct sub-pixel
/// phases, so twelve keys; all cached, so no tile touches the sheet.
pub const MEDIAN_PAGE: Archetype = Archetype {
    name: "median page",
    commands: 12,
    distinct: 9,
    segments: 8,
    side: 11.0,
    recorded: Some(row(12, 0, 9, 12, 0, 0, 0, 0, 0, 0)),
    ..BLANK
};

/// A dense page of text at the corpus's 99th percentile — and at its *measured* reuse,
/// which is five placements per outline rather than the fifty-five a fixture built from
/// the brief's one page assumes.
///
/// **Recorded row.** 4 320 placements over 818 outlines collapse to 2 164 keys, which is
/// the quantised phase doing its job.
///
/// **40 tiles and 8 956 coverage texels are its two curve clips, cut around their
/// marks.** Each clip takes a run of twenty consecutive marks, so its box is twenty
/// cells wide and one tall — larger than any mark under it, a thirtieth of the page —
/// and every one of the forty clipped commands rasterises a tile that the residue then
/// multiplies into. **No region is kept**, and that is ADR 0049's admission rule working
/// rather than failing: the chain's box costs more than the twenty small tiles it would
/// serve, which is the clause of that ADR written for exactly this shape — a `q W n`
/// around a line of text. (Before 2026-08-17 this row read 40 tiles and **2 regions** for
/// a page whose clips met **0 of 40** marks; `doc/notes-clipped-instrument.md` §3.)
pub const DENSE_TEXT: Archetype = Archetype {
    name: "dense text",
    commands: 4_320,
    distinct: 818,
    segments: 12,
    side: 11.0,
    clips: 2,
    clipped: 40,
    recorded: Some(row(4_320, 0, 818, 2_164, 1, 40, 0, 0, 40, 8_956)),
    ..BLANK
};

/// Artwork: strokes beside fills, clips on most of it, a few blended groups. The shape
/// of the Illustrator and `InDesign` pages that carry every group feature we have.
///
/// **Recorded row.** 684 top-level nodes are 676 draws plus 8 groups
/// (`Counters::commands` counts the scene's top level). **3 layer textures** are the
/// root's accumulator, one group's at a time, and the copy of the pixels that group's
/// composite covers — ADR 0020's depth pricing showing its work on eight sibling groups,
/// at ADR 0038's one texture per plan.
///
/// **600 tiles, 3 542 360 coverage texels, and both halves of ADR 0049 on one page.**
/// Every one of the 600 curve-clipped commands meets its clip and rasterises a tile of
/// about 5 900 texels; of the 185 chains, **66 keep a region** — cut around three or four
/// marks in one line, it costs less than the tiles it serves — and the rest are refused
/// one and rasterise per tile, **384** times, which is the wrapped runs whose box is the
/// width of the page's grid. 66 + 384 = 450 rasterisations where the page has 600 clipped
/// commands, and that difference is what ADR 0049 buys.
pub const ARTWORK: Archetype = Archetype {
    name: "artwork",
    commands: 900,
    distinct: 300,
    segments: 24,
    side: 60.0,
    strokes: 405,
    clips: 185,
    clipped: 600,
    groups: 8,
    blended_groups: 4,
    recorded: Some(row(684, 0, 300, 300, 1, 600, 3, 66, 384, 3_542_360)),
    ..BLANK
};

/// A page of photographs: the corpus's 99th percentile for image placements, over text.
///
/// **Recorded row.** 200 fills and 32 images under *rectangular* clips: **no tiles at
/// all**. Where dense text's clips leave a residue this one's resolve to a rectangle,
/// which is ADR 0007's whole claim and the reason `rect_clips` is a field.
pub const IMAGE_PAGE: Archetype = Archetype {
    name: "image page",
    commands: 200,
    distinct: 60,
    segments: 8,
    side: 11.0,
    images: 32,
    image_side: 128,
    clips: 4,
    clipped: 32,
    rect_clips: true,
    recorded: Some(row(232, 0, 60, 158, 4, 0, 0, 0, 0, 0)),
    ..BLANK
};

/// The corpus's clip mountain, at a fifth of its size: the page that prompted it defines
/// **15 004** clip regions, and 3 000 costs the same lanes in a suite that has to finish.
/// Nothing here was invented — a page like it exists, and it is why
/// `clip_distinct_regions` is a counter rather than a hope.
///
/// **Recorded row.** Twelve hundred rectangular clips resolve to twelve hundred distinct
/// regions and cost **nothing else**: no tile, no layer, nothing culled. The 800 atlas
/// keys are the 1 200 placements over 200 outlines collapsing by phase.
pub const CLIP_MOUNTAIN: Archetype = Archetype {
    name: "clip mountain",
    commands: 1_200,
    distinct: 200,
    segments: 8,
    side: 24.0,
    clips: 1_200,
    clipped: 1_200,
    rect_clips: true,
    recorded: Some(row(1_200, 0, 200, 800, 1_200, 0, 0, 0, 0, 0)),
    ..BLANK
};

/// The corpus's largest page, scaled down: that page holds **66 309** commands over
/// 65 978 distinct outlines. What distinguishes it is not its size but its **reuse of
/// exactly one** — every command carries its own outline, so the atlas never answers and
/// every command rasterises — and 1 500 commands hold that property while leaving a debug
/// build able to finish (an unoptimised rasteriser is twenty times slower, and the
/// archetype gate runs on every `cargo test`).
///
/// **Recorded row.** Against dense text's 5.3 placements per outline, this is the other
/// end of the corpus.
pub const GIANT: Archetype = Archetype {
    name: "giant",
    commands: 1_500,
    distinct: 1_500,
    segments: 8,
    side: 9.0,
    rect_clips: true,
    recorded: Some(row(1_500, 0, 1_500, 1_500, 0, 0, 0, 0, 0, 0)),
    ..BLANK
};

/// A drawing: tens of thousands of small filled polygons, each its own outline, each
/// carrying fifty-odd path segments, and no text, no image, no group and no clip
/// anywhere on the page.
///
/// **The caller's own file, scaled down.** That page is 49.7 MB and one content stream:
/// **58 009 commands — 58 003 fills, six strokes — over 3 011 879 path segments, 51.9 a
/// fill**, and at its fit view a mark is about three device pixels across
/// (`pdf-viewer/doc/QUORRA_ENCODE_THREADS.md` §1). It is a geological cross-section
/// exported by Inkscape, and it is every drawing, map, plan and chart in a corpus.
/// [`CALLERS_DRAWING`] is that page at its own size and count.
///
/// What distinguishes it from [`GIANT`] — which also reuses exactly one outline — is the
/// **ratio**: fifty-two segments flattened for a nine-pixel tile, where giant flattens
/// eight for eighty.
///
/// **Recorded row.** **1 194 keys and 6 tiles** is the six strokes, which have no atlas
/// at all — a stroke's coverage is its *expansion*, not its outline — and the 1 194
/// fills, whose three-pixel tiles the atlas takes. Otherwise the counters cannot tell
/// this page from giant, and that is worth saying rather than hiding: what differs is the
/// *segments* behind the numbers — 62 400 against giant's 12 000, for a ninth of the tile
/// area — and `Counters` has no field for it. Its **245 coverage texels** are those six
/// strokes' expansions and nothing else.
pub const DRAWING: Archetype = Archetype {
    name: "drawing",
    commands: 1_200,
    distinct: 1_200,
    segments: 52,
    side: 3.0,
    strokes: 6,
    recorded: Some(row(1_200, 0, 1_200, 1_194, 0, 6, 0, 0, 0, 245)),
    ..BLANK
};

/// The seven archetypes, as `tests/archetypes.rs` gates them.
pub const ARCHETYPES: [&Archetype; 7] = [
    &MEDIAN_PAGE,
    &DENSE_TEXT,
    &ARTWORK,
    &IMAGE_PAGE,
    &CLIP_MOUNTAIN,
    &GIANT,
    &DRAWING,
];

// ---------------------------------------------------------------------------
// Pages an instrument draws that are not archetypes. Each is named here because
// it was a copy in an example whose comment claimed it was one of the seven.
// ---------------------------------------------------------------------------

/// **The caller's page at its own size**: 58 009 commands over 58 009 outlines, six of
/// them strokes, on the 900 × 1100 window their trace measured, where a mark is about
/// three device pixels across.
///
/// [`DRAWING`] is the same shape scaled to 1 200 commands so that a debug build can
/// finish it; this one is the page itself, and only `examples/encode_threads.rs` draws
/// it — a thread sweep is the one measurement for which the scaled version would be
/// answering a different question.
pub const CALLERS_DRAWING: Archetype = Archetype {
    name: "caller's drawing",
    width: 900,
    height: 1100,
    commands: 58_009,
    distinct: 58_009,
    segments: 52,
    side: 3.0,
    strokes: 6,
    ..BLANK
};

/// [`DENSE_TEXT`] **without its two curve clips** — and it is a different page.
///
/// `examples/encode_threads.rs` has drawn this since ADR 0054 while its comment said it
/// was "`tests/archetypes.rs`'s dense-text row". It is not: the archetype places 40 of
/// its 4 320 commands under curve clips, and those 40 are the marks that do not divide
/// across encode threads. The difference was invisible while each example held its own
/// copy of the generator, and naming it is what this register is for.
///
/// **Nothing about it is changed here.** ADR 0054's thread sweep was measured on this
/// page, and re-cutting it would invalidate that measurement in the same round that
/// moved it — which is the trap `doc/notes-clipped-instrument.md` §3.4 names.
///
/// **Whether the sweep should run on the archetype instead was measured on 2026-08-23 and
/// declined**, and the reason is that the two pages differ by less than the sweep can
/// read: the archetype's 40 residue-clipped marks are 8 956 coverage texels against an
/// atlas working set of 476 892 that the clips leave untouched — **1.84 %** of its
/// coverage work, and 0.25 % of the 3 542 360 texels [`ARTWORK`] already contributes to
/// the same column. `examples/encode_threads.rs`'s `SHAPES` carries the decision and the
/// noise figure it is measured against.
pub const DENSE_TEXT_UNCLIPPED: Archetype = Archetype {
    name: "dense text, unclipped",
    commands: 4_320,
    distinct: 818,
    segments: 12,
    side: 11.0,
    ..BLANK
};
