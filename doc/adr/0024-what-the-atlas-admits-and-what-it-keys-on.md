# ADR 0024 — What the atlas admits, and what it keys on

Status: accepted, 2026-08-11. Takes `doc/PLAN.md`'s recorded atlas-policy question,
which the command-culling work uncovered and deferred. Fixes a correctness defect found
while taking it.

## Context

`MAX_GLYPH_DIM = 128` decided what the glyph cache would hold: a fill whose device
bounds exceeded 128 pixels on either side never entered the atlas and took the scratch
path, rasterised again on every frame. Its comment says what the bound was for — "the
bound keeps one zoomed-in letterform from evicting a page's worth of text".

**The bound is a dimension and the thing it protects is a budget.** That mismatch is a
cliff, and `examples/zoom` prices it. The dense page held at one magnification, RADV,
fastest of five (the machine carried a load average of 20 during both runs, so read the
columns against each other rather than as absolutes):

| zoom | visible glyphs | encode before | encode after |
|---|---|---|---|
| 8× | 154 | 0.896 ms | 0.689 ms |
| 12× | 74 | **13.637 ms** | **0.652 ms** |
| 16× | 46 | 9.863 ms | 0.617 ms |
| 20× | 30 | **19.413 ms** | **0.498 ms** |
| 100× | 15 | 7.557 ms | 4.806 ms |
| zoom sweep, worst frame | — | 35.763 ms | 16.179 ms |

A frame drawing thirty glyphs cost twenty times one drawing 5 933, because the thirty
were re-rasterised every frame and the 5 933 were not.

## Decision

### 1. Admission is a share of this atlas, not a constant

`AtlasStore::admits(width, height)`: the tile must fit the texture — a wider one can
never be packed — and take no more than `MAX_TILE_SHARE` (an eighth) of it. Against the
default 8 MiB atlas that is a 1 MiB tile, about 1024×1024, so eight such tiles fill the
cache and a ninth evicts. The protection is now stated against the quantity it was
always about, and it scales with `Options::atlas_budget` instead of ignoring it.

A tile the rule refuses still draws, uncached, through the scratch path. Nothing about
correctness depends on the answer — `the_two_paths_draw_the_same_pixels` holds that.

### 2. A repack only happens when it would help

Pressure — a tile that did not fit — used to reset the atlas unconditionally after the
frame. That is right when the atlas holds a *previous* page's tiles and this frame's
working set would fit. It is wrong when the frame's own distinct keys are simply larger
than the cache: resetting throws away the part that fits and hits, and every frame pays
the packing again. Measured at 100× on the ladder, where it cost 6.0 ms of encode
against 4.8 ms for keeping what fits.

So the encoder reports `atlas_requested_bytes` — the bytes this frame's *distinct* keys
asked for, hits included — and the device resets only when that would fit the atlas.

### 3. The fill rule is part of the key

**This is a correctness fix and it is the more important half of this ADR.**
`GlyphKey` was `(outline, linear part, phase)`. The same outline under §8.5.3.3's two
rules is two different pictures wherever a subpath nests — non-zero fills it, even-odd
holes it — so a cache without the rule hands the first request's tile to the second.

It was invisible because the dimension bound kept most such shapes out of the atlas:
the suite's own fill-rule test uses a 140-pixel shape, twelve pixels past the old cap,
and it began failing the moment admission changed. `tests/atlas_policy.rs` now holds it
at 70 pixels, where it would have failed before this ADR too — verified by putting the
defect back for one run.

### 4. `Counters::tiles` counts

It was documented "M5; 0 until then", M5 shipped three milestones ago, and nothing ever
set it: a frame that packed forty tiles reported zero. It now counts what the sheet
holds, from the one door onto it, so the admission rule above is observable from outside
rather than inferred. A `Frame` must not carry a number that is not true.

## What it does not do

**No recency, still.** Entries carry no last-used stamp, so the atlas cannot evict a
cold tile to make room for a warm one; what it does now is decline to throw everything
away when that would not help. Recency is the next question and it wants its own
measurement — the admission rule changed which pages reach pressure at all, so measuring
it before this would have been measuring a different atlas.

**A zoom gesture is unchanged**, and that is inherent: every frame of a gesture is a new
scale, so every key is cold whatever the policy. What this ADR fixes is *holding* at a
magnification, which is what a reader does.

## Revisit when

A page thrashes the atlas — many distinct tiles each near the share. The instrument is
`Counters::tiles` rising on a page whose glyphs repeat, and the answer then is recency
rather than a smaller share.
