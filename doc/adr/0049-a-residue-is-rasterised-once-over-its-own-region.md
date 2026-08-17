# 0049 — A clip chain's residue is rasterised once, over the region it occupies

Date: 2026-08-15. Status: **accepted, and built — with a correction to what its numbers
were taken on**, 2026-08-17, at the end of this file. The mechanism and the saving are
unchanged and were re-measured; **the fixture that demonstrated them was weaker than this
ADR says it was**, and two of its sentences about which chains are admitted are wrong for
the page that now exists.

Takes `HANDOVER.md`'s item 2 — the only work left in the tree with milliseconds in it —
and finishes what ADR 0030 started. That ADR read ISO 32000-2 §8.5.4 as saying a clip
chain is **one region**, and drew from it how the links combine (`min`, not a product).
The same sentence has a second consequence, which nothing acted on until now: a region
does not depend on which mark is asking about it, and so has no business being
rasterised once per mark.

Along the way this ADR fixes a defect in the rasteriser that no test could see and that
the region could not be built over without it. That half is first, because it is the
enabling half.

## Context

`Encoder::residue_intersection` was called once per clipped command, from three sites
(`encode.rs`'s `coverage_tile` and `plan_group_residue`, `encode/rare.rs`'s image lane),
and each call flattened every non-rectangular link of the chain and rasterised it over
**that command's own tile**. The artwork archetype — the corpus's p99 clip shape, 185
curve clips and 600 commands under them — did that 600 times for 185 regions.

What that is worth was measured before anything was designed, with a temporary probe
counting flatten and `fill_mask` spans and pixels through `tests/archetypes.rs` (load
average 14–50, so the ratios are the evidence and the milliseconds are context):

| archetype | flatten calls | fill pixels | of which residue |
|---|---:|---:|---:|
| dense text | 2 244 | 494 804 | 40 calls, 8 956 px |
| **artwork** | 1 500 | 8 893 166 | **600 calls, 3 577 552 px** |
| clip mountain (rectangular clips) | 800 | 772 740 | none |

The residue span was **17.3 ms of the 65.6 ms** of flattening and rasterising that
artwork's encode does — 26 %, not the 100 % that `PLAN.md`'s "35 ms of a 43 ms frame
re-rasterising the same residue coverage" implies. That sentence was reading the whole
geometry phase as though it were all residue; it is about a quarter of it, and this ADR
corrects the claim as well as the cost.

The same probe rules out the cheap version of this change: **flattening is 1.9 µs a call**,
so the 600 residue flattens are ~1.15 ms — 6.6 % of the residue span. Memoising the
flatten alone, which is the change with no fidelity question attached to it at all, was
worth about 1.5 % of geometry. It is not the answer and it is not taken.

## The defect this found: a tile's border column got somebody else's coverage

A region can only serve a tile if the tile is a *window* on it. The first thing built was
therefore the probe that asks whether it is:  forty closed curves, each rasterised over
its own bounds and then over twenty tiles cut out of it. It read **2 684 differing pixels
of 2 863 228, the worst by 185 of 255** — which is not rounding, and not a difference the
cache would have been allowed to ship.

The cause is in `raster::deposit_slab`, and it is one line of intent that does not hold:

> geometry left or right of the region clamps to its border (winding preserved, position
> clamped)

Clamping the two *endpoints* of a slab piece and interpolating between them preserves the
row's total winding — every column past the crossing reads the same value, which is why
nothing downstream ever saw this — but it spreads the height the piece spent outside the
region across the columns just inside it. A piece running from `x = −25` to `x = +2`
covers column 0 for almost its whole height; clamped, it is drawn as though it crossed
columns 0, 1 and 2 evenly.

**Decision: cut the piece at the border instead of clamping its ends.** Each part is then
wholly inside or wholly outside; the outside part deposits its winding at the border
column, for exactly the height it spends there. The cut runs only when an endpoint is
outside, so a piece wholly inside takes the same arithmetic it always did, to the bit —
which bounds what moves to *tiles that a clip or the page edge cuts in x*, and leaves
every other mark in the tree pixel-for-pixel where it was.

The value is now derivable without any rasteriser in the argument, which is what
`a_tile_whose_geometry_enters_from_outside_is_exact` asserts: one straight edge from
`(−2, 0)` to `(2, 1)` with the interior to its right covers 0.625 of the pixel
`[0,1] × [0,1]` — 0.5 where the boundary is left of the pixel, 0.125 where it crosses it,
0 where it is right of it — and `round(0.625 × 255) = 159`. The tree read **128**.

After the cut the same forty-curve probe reads **31 pixels of 2 863 228, every one by 1 of
255**. That residual is arithmetic rather than geometry and is not removable without
changing the accumulator: the region's prefix sum crosses the columns left of a tile one
at a time where the tile takes them as a single deposit at its border, and `f32` addition
is not associative. `a_tile_is_the_crop_of_the_region_that_contains_it` holds both the
1-of-255 bound and the rate.

## The decision

**A chain's residue is rasterised once, over the intersection of its links' device bounds
held to the target, and every command takes a window on it** — wherever keeping that
region is worth more than the tiles it replaces.

### The rule: a region must not cost more than the tiles it replaces

`encode::residue::ResidueRegions::admit` is the whole of the decision, and it is ADR
0029's finding — *a cache is worth what it is used for* — arriving with a different unit.
A region can be the whole page while the tile a command asks for is one small mark, so a
page-sized clip over forty small marks would pay two million pixels to save nine thousand.
Both halves are checked **before anything is allocated**:

- `region ≤ uses × tile`, where `uses` is counted from the scene before the walk and
  `tile` is the tile the first command asked for. Conservative in the direction that
  matters: an admitted region also replaces `uses` flattenings with one, so it is cheaper
  than the comparison says; a refused one is at worst exactly as expensive as before.
- `region` fits what is left of the frame's residue budget — a quarter of the caller's
  own `Options::max_frame_bytes`, so a caller who lowers that number for a small machine
  lowers this with it, and there is no constant here that the API cannot reach.

`uses` is counted by two integer passes over the scene: one over the commands (groups and
soft masks included) and one down the clip list carrying each clip's count into its
parents, which is exact because a parent's id is always smaller than its child's. Neither
pass touches a resource, so this cannot refuse a scene the walk would have drawn. Like
ADR 0029's census the count is an **upper bound**: a chain is keyed by its deepest
non-rectangular link, so where residue clips nest a shallower chain's count includes
commands that will ask for a different region.

### Exceeding the budget declines the cache; it does not refuse the frame

Principle 6 says a frame is drawn or refused, and this is the case where the two halves of
it point in the same direction: a frame that *could* be drawn and is refused because a
cache filled up is the failure, not the safeguard. There is a fallback that costs only
time — rasterising the chain per tile, which is what every frame did before this ADR — and
the atlas already sets the precedent, since a tile it will not admit takes another lane
rather than failing the frame (ADR 0029's table).

What the budget refuses is the **allocation**, before it happens, which is what principle
3 asks of it. `a_budget_too_small_for_a_region_still_draws_the_frame` renders the same
scene at a budget whose quarter cannot hold one region and holds the frame to drawing.

### Two counters, and they count keys

`Counters::clip_residue_regions` is the number of distinct regions rasterised and
`clip_residue_tiles` the number of residue rasterisations a single command's tile paid
for. Both are exact functions of the scene and the viewport, so both joined
`tests/archetypes.rs`'s signature, which compares by equality on any machine.

Keys and not a hit rate, for CLAUDE.md's reason: the caller's clip-mask cache once
answered all 303 lookups a page made and built 303 identical page-wide masks. A hit rate
here would read 100 % for a page of one clip and 100 % for a page of forty, and the second
is the page that pays forty times. `the_counter_is_of_regions_and_not_of_the_commands_that_ask`
draws both pages and holds them to **1 region / 0 tiles** and **0 regions / 40 tiles**.

## What it buys

`examples/residue_clip.rs` — the artwork archetype, headless, into a `Target::Texture`
created once, `instrument_encode` on, minima of twenty steady frames after a first frame
reported separately. Base commit and this tree, **alternating**, three rounds each, in one
hour on one machine.

| round | load | encode: geometry | encode | wall |
|---|---|---:|---:|---:|
| 1 base | 3.8 | 37.78 ms | 46.26 ms | 51.72 ms |
| 1 new | 4.8 | **32.42** | **40.98** | **43.97** |
| 2 base | 4.4 | 40.78 | 50.07 | 53.68 |
| 2 new | 4.3 | **29.02** | **37.50** | **40.35** |
| 3 base | 4.0 | 39.73 | 48.72 | 56.88 |
| 3 new | 3.8 | **28.89** | **37.17** | **42.18** |

Minima across the three: **geometry 37.78 → 28.89 ms (−24 %), encode 46.26 → 37.17
(−20 %), wall 51.72 → 40.35 (−22 %)**. The machine was quiet for all six runs, which is
the only reason these are worth quoting at all — the same pair measured at load 30 read
the wrong way round by 60 %, and that is `HANDOVER.md`'s first trap doing exactly what it
says it does.

The saving is smaller than the residue span because the windows are not free: 600 crops
of about 6 000 bytes each replace 600 rasterisations, and 185 regions are still
rasterised. What is removed is 415 rasterisations and 415 flattenings.

The counter is the part that is not a wall clock: **600 residue rasterisations become 185**,
`clip_residue_tiles` is 0, and `tiles`, `bytes_uploaded` and every other row of the
archetype signature are unchanged — which is what says this moved a cost and not a mark.

## What it costs, and what it does not fix

- **The three `ScratchExhausted` refusals at 4× are untouched**, and the evidence is a
  counter rather than an argument: what those pages run out of is *sheet height*, the
  sheet holds one coverage tile per clipped command, and `tests/archetypes.rs`'s `tiles`
  row is unchanged on every archetype. A residue region is a host-side allocation that
  never reaches the sheet. `HANDOVER.md`'s item 2 held two things and this ADR takes one
  of them.
- **A page whose clip is much larger than its marks gets nothing.** That is the admission
  rule working, and it is the common shape of a real `q W n` around a paragraph: dense
  text's two chains are admitted because its clips are small, and a page-sized clip over
  forty small marks is refused. The region is the chain's own bounds, not the union of the
  tiles that will ask for it — that union is not knowable at the first ask without a second
  pass over the commands, and a two-pass encode is what ADR 0034 declined for the tiling.
- **A frame holds its regions until the encode ends.** Up to a quarter of the frame budget
  of host memory, freed with the encoder. Nothing pools them across frames: the retained
  encode (ADR 0048) already answers the "same page again" question, and a cache that
  outlived its scene would be ADR 0029 §3's rejected memory in a second place.
- **The pixels of a clipped mark move**, by the border cut and not by the region. Where a
  clip's or the page's edge cuts a coverage tile in x, the border column now carries the
  area instead of a smear of it; everything else is bit-identical.

### The caller's corpus

One copy of their tree, all four runs inside it, flipping only this worktree's source
between the base commit and the change — and both runs of each pair in the same hour, as
`HANDOVER.md` insists, because that tree moves under us.

| | verdicts before | verdicts after |
|---|---|---|
| scale 1 | 930 agree / 24 differ / **2 refused** / 18 not comparable | **931** / **23** / 2 / 18 |
| scale 4 | 936 agree / 10 differ / **5 refused** / 23 not comparable | 936 / 10 / **5** / 23 |

**No refusal moved at either scale**, which is the claim above about the sheet, measured
rather than argued. One page changed its verdict and it changed toward the oracle:
`issue2177.pdf` at scale 1 stops differing (it read `mean 1.1168, worst tile 7.14 at
(224, 160), SSIM 0.99719`). Two more pages moved their numbers and nothing else in either
run did — every other page's mean, worst tile, differing fraction and SSIM are identical
to the digit:

| page | scale | before | after |
|---|---:|---|---|
| `issue2177.pdf` | 1 | mean 1.1168, worst 7.14 | **agrees** |
| `issue6081.pdf` | 4 | mean 0.0038, worst 9.17 | mean **0.0037**, worst **8.86** |
| `issue11473.pdf` | 1 | mean 0.1003, worst 10.04 | mean 0.1004, worst 10.07 |

Two of the three moved toward the oracle and the third by a ten-thousandth of a mean
stated in unorm steps. That direction is expected rather than lucky: `tiny-skia` computes
each pixel's coverage from the geometry that reaches it, so a border column carrying the
area agrees with it where a smeared one did not — and the third page is the kind of
hundredth-of-a-step wobble ADR 0047 recorded on the same instrument, where a coverage byte
was already at a rounding boundary.

## Revisit when

- **A page is found whose regions the admission rule refuses and which spends real time
  in the residue.** The next lever is growing a region to the union of the tiles that ask
  for it, re-rasterising when a tile arrives outside it. The border cut is what makes that
  a performance decision rather than a fidelity one — every region agrees with every other
  to within the 1-of-255 above — and it is why that idea is recorded here rather than
  built: nothing measured yet asks for it.
- **The accumulator's arithmetic changes.** A fixed-point accumulator would make a tile
  the crop of a region *exactly*, and the 1-of-255 residual would go with it. That is a
  change to the hottest loop in the tree and nothing needs it today.

## Correction, 2026-08-17 — what the fixture demonstrated, and what it did not

**The decision is unchanged, the mechanism is unchanged, and the saving is real.** A
chain's residue is rasterised once over its own region and cropped per mark; the
admission rule is what it says; the border cut and its corpus evidence are untouched.
What is wrong is the **page** every number in "What it buys" was taken on, and two
sentences that describe which chains that page admitted.

`tests/archetypes.rs` placed a curve clip at `position(j, side × 6)` and the marks under
it at `position(i, side)` — two grids of different step — so **8 of artwork's 600 clipped
commands and 0 of dense text's 40** had a mark whose box met the box of the clip clipping
it. ADR 0057 found this by taking away the tiles those marks were getting for nothing
(`doc/notes-tiling-bound.md` §3); the fixture was re-cut on 2026-08-17
(`doc/notes-clipped-instrument.md`).

What follows for this ADR, item by item:

- **"600 residue rasterisations become 185" stands as arithmetic and was measured on the
  wrong page.** 585 of those 600 rasterisations were of a chain that admitted no pixel of
  the mark asking, and 177 of the 185 regions served no mark at all. The saving was real —
  the work removed was real work — but it was **the removal of repeated rasterisation of
  tiles that were then multiplied by zero**, which is not the case this ADR exists for.
- **The 37.78 → 28.89 ms of geometry is a number about that page** and is not comparable
  with any number taken on the page that exists now. `examples/residue_clip.rs` copies the
  archetype, so it was the same page; it has been re-cut with it.
- **"dense text's two chains are admitted because its clips are small" is now false, and
  it was true only because the clips were mark-sized by accident.** A clip cut around the
  twenty marks it clips is *larger* than any of them, and the admission rule refuses it:
  dense text now reads **0 regions and 40 per-tile rasterisations**. That is the rule
  working exactly as the sentence beside it describes — "a page whose clip is much larger
  than its marks gets nothing" — and it is worth having on a page in the tree.
- **Both branches are now exercised by one page.** Artwork reads **66 regions and 384
  per-tile rasterisations** against 600 clipped commands: a run of three or four marks on
  one line keeps its region, and a run that wraps to the next line has a box the width of
  the grid and is refused one. 450 rasterisations where the page has 600 clipped commands
  is this ADR's mechanism, measured on marks that are actually clipped.
- **The 8 956 residue pixels the Context table records for dense text are exactly what the
  re-cut page's `coverage.texels` now reads** — the same number from an independent
  direction, five days and two ADRs apart, which is the one piece of evidence in this file
  that the re-cut did not disturb.

The general lesson is `doc/HANDOVER.md`'s trap about fixtures that do not overlap, arriving
a second time in the same tree: **a gate on an interaction must fail when the interaction
stops happening.** `tests/archetypes.rs` now has one that does.
