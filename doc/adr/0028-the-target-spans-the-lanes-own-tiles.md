# ADR 0028 — The winding target spans the lane's own tiles, and the lane is the one the atlas leaves

Status: accepted, 2026-08-12. Takes the piece ADR 0027 recorded as what it did not fix,
repairs a defect that ADR shipped with, and replaces its measured constant with the
question the atlas already answers.

*Extended, 2026-08-12 (ADR 0029), in both of the costs recorded below.* The first — a
page whose outlines are each placed once — is now part of the criterion: the lane asks
what the cache will *do* with a tile rather than what it will accept. The second — that
`AtlasStore::admits` answers a question about size and not about room — was implemented,
measured on four page shapes and the caller's corpus, found to move nothing, and removed;
ADR 0029 records the numbers, and the reason is that the first fix makes the second
unnecessary.

## Context

ADR 0027 cut the winding target into horizontal bands, which bounded its *height*. Its
width stayed the scratch sheet's, and the sheet is shared with the CPU lane and packed
across the device's full dimension: thirty shapes of 1 200 pixels land on shelves of a
sheet 15 600 texels wide, so one shelf-band was still 194 MB and the frame was refused on
a lane whose entire purpose is large shapes. That ADR named the fix — "for the target to
span the GPU lane's own tiles rather than the sheet's full width" — and left it.

It also left something nobody knew was there. **Every band after the first drew
nothing.** `fs_winding` tests a fragment against its own tile's rectangle, stated in
sheet texels; the fragment's coordinate is in the attachment, which a band's viewport
had already moved by the band's first row. For band 0 the two agree by accident. For
every later band every fragment of every tile failed the test and was discarded, and the
frame came back `Ok` with holes in it — principle 6's exact failure, and the third time
this project has met this specific agreement (the caller's `QUORRA_FEEDBACK.md` §11 is
the first two). No test saw it because every fixture in `tests/coverage_lanes.rs` drew a
single band, and — as this ADR's measurement then found — most of them were not on the
GPU lane at all.

## Decision

### 1. The target is a pane: a rectangle over this lane's tiles, bounded in both axes

A [`Pane`] is a run of tiles and the rectangle that contains them. The target holds one
at a time, and `crate::pane` chooses them in the order that makes the bound hold:

- **The target's extent is fixed first**, from `PANE_BYTES` (16 MiB) and the tiles' own
  sizes: at least the largest tile, then the remaining budget spent on width, then on
  height. Panes are cut to fit *inside* it.
- **Width before height**, because the sheet is shelf-packed and consecutive tiles in
  sheet order sit side by side. When the sheet is narrower than the width budget the
  leftover goes to height, and a pane becomes exactly ADR 0027's band — the old
  behaviour is the special case, not a separate path.
- The one way past the budget is **a single tile larger than it**, because a tile is
  never split. Thirty shapes of 1 200 pixels now need a target of 1 343 × 1 560 — 16.7 MB
  where a band asked for 194.

Choosing panes first and measuring them afterwards would not have bounded anything: the
target is one texture, so it costs the largest pane in *each axis*, and a tall narrow
pane beside a short wide one costs their maxima multiplied. Fixing the extent first is
what makes the budget a budget.

### 2. Three places subtract the pane's origin, and the third was missing

`vs_winding` subtracts it before mapping to clip space, **`fs_winding` adds it back to
test a fragment against its tile**, and `fs_resolve` subtracts it again when it reads the
target. Each is named in the shader beside the subtraction, with what happens if it is
the one left out. `tests/coverage_lanes.rs` now draws a grid whose panes are offset in
both axes, so no one of the three can be dropped without a test going red.

### 3. Each pane draws its own tiles' triangles

ADR 0027 had every band draw every vertex in the frame and let the shader map the
outsiders out of clip space — affordable when a band was a shelf, ruinous when a pane can
be one large tile, since the frame would pay its whole vertex buffer once per tile. A
tile now records where its vertices went (the encoder appends them in one run anyway), a
pane carries its tiles' ranges coalesced, and in sheet order they coalesce back to a
single draw. The vertex buffer is still never permuted.

### 4. The lane is the one the atlas leaves, not the one a constant names

ADR 0027 measured a crossover in tile area — half a megapixel — and asked its successor
to re-derive the table rather than inherit the constant. Re-deriving it found that tile
area is the wrong axis. What the CPU lane has that the device has not is the **cache**: a
tile the atlas admits is rasterised once and reused by every later placement and every
later frame, and no lane competes with not doing the work. A tile the atlas refuses is
rasterised into the scratch sheet on every frame, and there the device wins at every size
measured.

RADV, sixteen samples, a 3 600 × 3 600 page, texture target, fastest of nine frames,
milliseconds, lane forced either way (`tests/lane_crossover.rs`):

| tile | texels | atlas holds it — CPU / GPU | atlas refuses it — CPU / GPU |
|---|---|---|---|
| 50 × 65 | 3 250 | **1.0** / 20.2 | 54.8 / **21.2** |
| 200 × 260 | 52 000 | **0.4** / 16.0 | 35.5 / **15.0** |
| 500 × 650 | 325 000 | **0.3** / 9.9 | 32.8 / **13.7** |
| 700 × 910 | 637 000 | **0.2** / 11.1 | 26.0 / **12.6** |
| 900 × 1 170 | 1 053 000 | **0.4** / 13.3 | 33.9 / **15.0** |
| 1 200 × 1 560 | 1 872 000 | — | 32.1 / **9.6** |

Twenty to sixty times the wrong answer on the left, two to three times on the right, and
**the same 52 000-texel tile appears in both columns** — so no area separates them.
`GPU_LANE_MIN_AREA` is deleted, `AtlasStore::admits` takes its place, and ADR 0026's triangle
comparison stays beneath it as the floor that keeps a nine-pixel glyph off this lane
whatever the atlas says. ADR 0027's 512 KiB sat *below* the admission threshold of the
default 8 MiB atlas, which is how one constant was wrong in both directions at once: it
took 512 KiB–1 MB tiles the atlas would have cached (700 × 910: 12.6 ms against 0.2), and
withheld the lane from everything smaller that the atlas had refused (200 × 260: 35.5 ms
against 15.0).

## What it buys

| | before | after |
|---|---|---|
| 30 shapes of 1 200 px | refused, 309 MB | drawn, 16.7 MB of target |
| a frame of two or more bands | **part of the page silently blank** | drawn |
| 8 shapes of 800 px, 30 MB budget | refused (41 MB band) | drawn (16.7 MB pane) |
| a page of 200 px tiles, no room in the atlas | CPU 35.5 ms | **GPU 15.0 ms** |
| a page of 1 200 px tiles | CPU 32.1 ms | **GPU 9.6 ms** |
| a page of cached glyphs under `Coverage::Gpu` | 11-16 ms on the device | **0.4 ms** in the atlas |

The caller's 974-page corpus gate is unchanged — 917 agree, 35 differ, 5 refused, and
5.01 s of rasterisation against HEAD's 5.01 s — which is what it should be: they render
on `Coverage::Cpu`, and a lane criterion that moved their verdicts would have been a
defect in the shared path rather than in this lane.

**On `Coverage::Gpu` that corpus is the honest counterweight to the table above.**
Forcing the GPU lane on all 957 pages, same working copy: HEAD draws them in 4.42–4.49 s
with 914 agreeing, 37 differing and **6 refused**; this ADR draws them in 4.72–4.83 s with
908 agreeing, 44 differing and **5 refused**. Each of those movements was attributed by
re-running with one half of this ADR at a time, because "more pages differ" is the kind of
number that must be explained rather than accepted.

**The pane rewrite (§1–3) changes no page's pixels but one, and un-refuses another.** With
panes in place and ADR 0027's constant still choosing the lane, 36 pages differ instead of
37 and the single change is `issue9418.pdf`, which **stops** differing — at HEAD it was
`mean 4.76, worst tile 191 of 255`. A 191 is not antialiasing; it is the blank-band defect
on a real document, and nothing else in the corpus was touched. The refusal that goes away
is `issue1905.pdf`, refused at HEAD for *400 681 916 bytes against a budget of
268 435 456* — the sheet-wide winding target, on a page from the caller's own corpus.

**The nine newly differing pages are the criterion (§4), and they are antialiasing.**
`bug1743245`, `bug1863910`, `bug1883609`, `issue16500`, `issue17492`, `mixedfonts`,
`textfields`, `transparency_group`, `vertical` — worst per-tile differences of **1.5 to
12.2 of 255**, means of 0.10 to 3.06, SSIM 0.978 to 0.996. Each is a page where the
criterion now hands tiles to the device that the constant kept on the processor, and the
device answers with sampled coverage instead of analytic: ADR 0016 states that difference
and `tests/coverage_lanes.rs` bounds it at 32 of 255 on a straight edge and 96 on a curve,
so every one of these is well inside what the lane promises. `knockout_groups_test.pdf`
moves the other way and agrees.

The time and the differences both land on a page shape this lane was not built for — real
corpus pages at 1× are small text, where the atlas is simply the right answer. The lane
exists for large shapes and magnified pages, and that is what the table above measures.

## What it costs, and does not fix

**A page whose outlines are each placed once pays the CPU lane where the device would
have been faster.** With the default atlas and every outline distinct — the corpus
median is 1.33 placements per outline, so this is the normal state and not a pathology —
the atlas admits the tiles, so the criterion sends them to the CPU lane: 38-44 ms cold
against the 13-20 ms the forced GPU lane took. Warm they are within noise of each other,
because by then the atlas has them. The criterion is deliberately blind to *how many
times* an outline will be placed, and the scene knows: counting placements per outline in
a pre-pass would let the lane be chosen on cache value rather than cache eligibility.
That is the next question on this lane, and it now has a number attached. *(Taken the
same day, ADR 0029: the first frame of such a page went from 40.3 ms to 14.7.)*

**`AtlasStore::admits` answers a question about size, not about room.** A page of 3 960
distinct 50-pixel glyphs asks for 12.9 MB of an 8 MiB atlas: every tile is admissible,
a third of them do not fit, and those fall back to CPU scratch rasterisation rather than
to this lane. Making the miss fall through to the GPU lane needs the lane decision to
happen after the atlas attempt rather than before it, which is a larger change than this
ADR takes. *(Tried and rejected, ADR 0029: with placement counts in the criterion the
atlas stops filling with tiles nobody reuses, and asking about room then moved nothing on
four page shapes or on the corpus.)*

**A pane is still cut greedily in sheet order.** Tiles that the shelf packer placed far
apart make a pane wider than its tiles need. Nothing measured says this costs anything
yet; it is written down so the next person does not mistake it for a design.

## Revisit when

Both of the first two items are settled by ADR 0029 — one taken, one measured and
refused. What is left here is the third: a pane cut in sheet order rather than by what
would pack tightest. Re-derive any of it with `tests/lane_crossover.rs` rather than
reasoning from this table, since the criterion has now moved three times.
