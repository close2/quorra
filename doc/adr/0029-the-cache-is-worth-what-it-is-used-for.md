# ADR 0029 — A cache is worth what it is used for, not what it will accept

Status: accepted, 2026-08-12. Takes the first of the two costs ADR 0028 wrote down,
measures the second, and rejects it.

## Context

ADR 0028 replaced a hand-picked tile area with the question the atlas already answers —
*will you hold this?* — and recorded what that still got wrong:

> A page whose outlines are each placed once pays the CPU lane where the device would
> have been faster. […] The criterion is deliberately blind to *how many times* an
> outline will be placed, and the scene knows.

The caller's corpus median is **1.33 placements per distinct outline**
(`doc/corpus-profile.md`), so a page whose shapes are each drawn once is the ordinary
page, not a pathology. For every one of them the atlas rasterises a tile, uploads it,
reads it once, and evicts it — the cache's whole cost and none of its benefit.

## Decision

### 1. The lane asks what the cache will *do*, not what it *allows*

`CacheProspect` is one answer to one question, assembled by the atlas and the scene
together and read by both lanes:

| the cache would… | the lane |
|---|---|
| refuse the tile's size (ADR 0024) | the device |
| hold it, and it is resident already | the atlas — a hit costs a lookup and a quad |
| hold it, and the page places it **once** | the device |
| hold it, and the page places it again | the atlas — the first rasterisation pays for the rest |

with ADR 0026's triangle floor beneath all of it, which is what keeps a nine-pixel glyph
off the device lane whatever the cache says.

Two things follow from asking it in one place. The prospect is computed **once per solid
fill** and read by the lane test and the glyph lane, so the two cannot disagree about the
same tile — a lane chosen on one reading of the cache and taken on another is how a tile
gets rasterised twice. And `GlyphPlacement` — the key, the integer origin and the
quantised phase — moves out of the glyph lane into the atlas that owns the key, because
the lane choice needs it before the glyph lane runs.

### 2. The census counts placements, and counts them loosely on purpose

`crate::census` walks the scene once before the encode walk and counts solid fills by
outline, linear transform and fill rule — everything the atlas keys on **except the
sub-pixel phase**. So two placements it counts together may still be two keys, and the
count is an *upper bound on reuse*.

The direction is the decision. "Placed once" is then a fact rather than a guess, and it
is the only conclusion drawn: over-counting sends a tile to a cache that might have been
slower, under-counting takes a tile away from a cache that would have served it many
times — and ADR 0028 measured that mistake at twenty to sixty times.

### 3. The answer depends on this frame alone

A version that remembered which keys the *previous* frame declined to cache was built and
measured: it would let a page draw on the device the first time it is seen and enter the
atlas the second, which is the best of both. Rejected, for a reason the timings alone do
not show — **the two lanes do not draw identical pixels**. A static page redrawn on a
scroll tick would have changed its antialiasing between the first frame and the third,
with nothing in the scene to explain it, and `tests/frame_independence.rs` exists because
that class of defect is the hardest kind to find. The numbers, for the record (200-pixel
tiles, distinct outlines, default atlas, RADV): with the memory 19.1 ms then 34.5 then
4.5; without it 15.5 then 13.1 then 11.6. It pays for itself after about six frames of
the same page, and it costs a picture that changes for no reason. Not worth it.

## What it buys

`tests/lane_crossover.rs`, RADV, 3 600 × 3 600, milliseconds, first frame / fastest of
eight after it. "Distinct" is the corpus median's shape — every outline placed once:

| tile | CPU lane | GPU lane, ADR 0028 | GPU lane, now |
|---|---|---|---|
| 100 × 130 | 47.3 / 13.5 | 47.9 / 13.3 (the atlas, unchanged) | **16.8** / 14.2 |
| 200 × 260 | 40.3 / 11.3 | 38.8 / 11.3 | **14.7** / 11.4 |
| 500 × 650 | 34.3 / 10.0 | 33.6 / 10.0 | **12.7** / 9.9 |
| 900 × 1170 | 41.2 / 17.1 | 37.5 / 17.3 | **13.7** / 10.3 |

The first frame — the one a person waits for when a page appears — is **2.5 to 3 times**
faster, and no later frame is slower. A page of one shared outline is untouched (0.4 ms
either way, every tile a cache hit), which is the case that had to stay untouched.

## What it costs

**Nothing measurable on the caller's corpus, and one page better.** 957 real pages on
RADV, same working copy, three runs each: with the census 4.72–4.76 s, without it
4.70–4.76 s — indistinguishable. The verdicts move by one page and in the right direction:
44 differ with the census, 45 without, the difference being `tiling_patterns_variations`,
which agrees with the oracle when the census is on. Real pages at 1× are text, where the
triangle floor keeps the device lane out of it and the atlas is simply right; the census
is for the pages the lane exists for — large shapes, and text at the magnifications §6 of
the brief cares about — and it is free on the rest.

**A page whose shapes are each placed once no longer fills the atlas.** That is the
intent, and the counter says so: `atlas_distinct_keys` counts what the atlas was *asked*
for, so it falls on such a page. The instrument still measures what it always measured —
the keys the cache was offered — and the ones the lane took instead were never offered.

**The walk itself costs 25 µs on a 5 933-command page**, against an encode of 80 — a
quarter of a phase this project measures in microseconds, and encode is on the startup
path §7 cares about. So it is taken **only under `Coverage::Gpu`**, which is the only
setting that reads it: `take_gpu_lane` answers `false` on sight under `Coverage::Cpu`, and
an empty census answers "not placed once" to everything, which is the lane every fill
would have taken anyway. The caller's default configuration pays nothing (measured: 79 µs
against 80 before this ADR).

## What was measured and rejected

**Asking whether the atlas has *room*, not merely whether it admits the size.** This was
ADR 0028's other recorded cost, and the fix is real: `admits` is about the tile and never
changes, while room is about the frame and every insert changes it, so a full atlas
promises a tile a home, rasterises it, refuses it, and hands it to the scratch path.
Implemented (a `has_room` mirroring the packer through one shared `fit`), measured, and
**removed** — it moved nothing anywhere:

| | with `has_room` | without |
|---|---|---|
| corpus, 957 pages, GPU lane | 4.72–4.76 s | 4.68–4.70 s |
| 3 960 distinct 50 px tiles, 8 MiB atlas | 30.1 / 24.6 / 21.7 | 28.2 / 25.0 / 22.1 |
| 972 outlines × 3 uses, 1 MiB atlas | 48.1 / 40.1 / 40.6 | 48.5 / 39.8 / 39.9 |

The reason is §1's third row: with the census in place the atlas no longer fills up with
tiles nobody reuses, so a full atlas is one full of tiles that *are* being reused — and
those are worth their space. Two mechanisms aimed at the same waste, and the cheaper one
got there first. The failed-insert fallback stays as the backstop it always was.

## Revisit when

The census's looseness is measured to matter — count the phase too and the answer
tightens, at the price of a census that depends on the viewport and the quantum. Or when
a caller renders one page for many frames at one zoom and wants the atlas to catch up
with it: that is §3's rejected memory, and it needs a way to change lanes without
changing pixels before it can be reconsidered.
