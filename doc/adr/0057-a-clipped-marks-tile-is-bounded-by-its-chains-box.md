# 0057 — A clipped mark's coverage tile is bounded by its chain's own box

Date: 2026-08-17. Status: accepted, and built. Takes `HANDOVER.md`'s item 2 and closes the
half of it that had milliseconds in it; `doc/notes-tiling-ceiling.md` is the measurement
that chose the candidate and `doc/notes-tiling-bound.md` is what building it found.

Two decisions, and they are one round because the second is the reason the first cost a
patched crate to measure:

1. **A rasterised coverage tile is sized by `shape ∩ clip ∩ target` where `clip` includes
   the box a residue chain's links admit** — not only the rectangular links.
2. **A refused frame accounts for the sheet it met, and a drawn one prices its coverage**,
   through one type, `CoverageSheet`, reported in both states.

## Context: where a tile's size came from, and the one line that decided it

`ClipResolver::resolve` walks a chain link by link. A link whose outline is an
axis-aligned rectangle under an axis-preserving transform intersects into the resolved
*rectangle* (ADR 0007) and costs a pixel nothing. Anything else became a `ResidueLink`,
and the rectangle was carried through untouched:

```rust
None => ResolvedClip {
    rect: current.rect,                    // ← untouched
    residues: Some(Arc::new(ResidueLink { clip: link, parent: current.residues.clone() })),
},
```

That is deliberate for what it says — a curve is not a rectangle and cannot *replace* the
clip rectangle — and it is silent about something else that is true: **a residue link
still has a bounding box.** `encode/clips.rs::chain_region` had computed one since
ADR 0049, and used it only to price the region cache. It never reached a tile.

So `coverage_tile` sized every clipped mark by `shape ∩ rect ∩ target`, and under a
residue-only chain `rect` is the open rectangle. A full-page mark under an
eighteen-pixel curve was charged a full-page tile, rasterised, shelf-packed, uploaded and
sampled — then multiplied by a residue coverage of zero.

`doc/notes-tiling-ceiling.md` §1 measured what that is worth on the page that refuses:
`bug1703683_page2_reduced.pdf` at 4× asks for **1 008 561 911 texels of coverage where its
141 chains admit 2 297 897** — a factor of **439**, with a *median chain box of 99 texels*
and forty-two chains eleven pixels wide charged 7 755 264 texels each. The adapter's
16 384-row sheet ceiling is only the limit that fires first.

## Decision 1 — the chain's box bounds the tile

`ResolvedClip` gains `residue_bounds`, and `ResolvedClip::mark_bounds()` is
`rect ∩ residue_bounds`. **Every rasterised coverage tile is sized by `mark_bounds`**;
`rect` is unchanged and stays what it always was — the rectangle the shader clips a quad
to, and the key `Counters::clip_distinct_regions` counts.

### Why removing those pixels removes nothing

ISO 32000-2 §8.5.4 makes a clip chain one region arrived at by intersection:

> After the path has been painted, the clipping path in the graphics state shall be set to
> the intersection of the current clipping path and the newly constructed path.

ADR 0030 read that as "the links intersect, they do not multiply"; ADR 0049 read the same
sentence one step further as "a region does not depend on which mark is asking". This is
the third consequence and it needs no new reading: **outside a closed link's own bounds
that link winds nothing**, so the chain's coverage there is zero, and the product a
clipped tile carries is zero with it. `chain_region` already states exactly this as its
reason for being an intersection.

### The box is the control hull, and that is the safe direction

The bound is computed where the link is already being examined, from
`encode/hull::HullMemo` — the memo ADR 0045 built for marks, keyed by `(outline, linear
part)`, which a page's clips share with its fills. Three consequences, and each is why the
hull was chosen over the flattened outline:

- **it costs no flattening at all**, and no second pass over the commands. A chain is
  resolved once per frame and memoised across shared prefixes, so this is one probe per
  newly-resolved residue link;
- **a page with no residue clip pays nothing** — the line is inside the `None` arm of the
  rectangle test, which such a page never takes;
- **it is an upper bound** on the curve by the convex-hull property of Béziers, and that is
  the only direction that is safe. A box the curve could leave would cut a mark. The
  flattened box `chain_region` computes is ≤ this one, so the region a chain is rasterised
  over is unchanged and a tile is at worst slightly larger than it needs to be.

A link whose outline has no points at all yields the **empty** box, which is a region that
admits nothing rather than a missing one: every mark under it draws nothing, which is what
the residue product already computed for it, and it now costs no tile.

### Where it applies, and where it deliberately does not

Three sites size a rasterised tile and all three take `mark_bounds`:
`encode/coverage.rs::visible_tile` (which `coverage_tile` now calls, so the CPU and GPU
lanes cannot come apart about a tile's extent — the ten lines of duplicated arithmetic
`doc/notes-encode-split.md` §5 named are gone), `encode/rare.rs`'s image lane, and
`encode/layer.rs::plan_group_residue`, where a group under one curve clip was rasterising
a page-sized mask.

`encode/device_space.rs`'s cull still tests `rect`. Culling by `mark_bounds` would be
correct and would drop more commands, but `Counters::commands_culled` is a caller-visible
number and the change belongs to whoever wants that saving, with its own measurement.

## Decision 2 — the two instrument debts

`doc/notes-tiling-ceiling.md` obtained every number in its §1 from `eprintln!`s in a
patched copy of this crate, because **no combination of the public API produces one of
them**. Two additions close that, and both are additive: no existing field changes type,
name or meaning.

### `CoverageSheet`, reported in both states

```rust
pub struct CoverageSheet { pub tiles: u32, pub texels: u64, pub width: u32, pub height: u32 }
```

- **`Counters::coverage`** on a drawn frame. `tiles` was a count of tiles and said nothing
  about how large one was, which is why two rounds in a row could measure a change this
  library's own signature could not see: a 402× reduction in coverage moved no row of
  `tests/archetypes.rs`. `texels` is that number, and it is a **count** rather than an
  occupancy ratio, for CLAUDE.md's reason — a ratio is a statement about the sheet you
  packed, never about the coverage you should not have asked for.
- **`RenderError::ScratchExhausted { limit, sheet, tile_width, tile_height }`** on a
  refused one. `limit` alone is a property of the adapter, so a caller could not tell "this
  page asks for a gigabyte of coverage" from "this adapter's textures are small". The same
  four fields, plus the tile that did not fit, make both diagnosable: which axis overflowed
  (`sheet.height + tile_height` against `limit`, with `sheet.width` usually nowhere near
  it), by how much, and how far the byte budget was from mattering.

**`ScratchPacker::reserve` now charges `placed` and `tile_area` only on success**, so the
sheet a refusal reports is what the frame *placed*. The candidate's width still raises the
shelf target for its own placement, and the candidate's area is still in the `√(2A)` sum it
is measured against, so no drawn frame's packing moves by a texel.

### What was considered and refused: a `Counters` on every refusal

The debt as `HANDOVER.md` states it is broader — "a refused frame has no `Counters` at
all". Attaching a whole `Counters` to a `RenderError` was refused, and the reason is
principle 6 rather than effort: a `Counters` from a half-finished walk is a set of numbers
about a frame that does not exist, and "whatever a `Frame` says about itself must be true"
does not become weaker when the frame is an error. What a refusal can honestly report is
the state of the thing that overflowed, which is what it now does.

There is also **no `Scene::cost()` answer here and there cannot be one**: the sheet's
height is a function of the *viewport*, which a `Scene` does not have. This is the one
budget principle 6's "discoverable before the frame" does not reach, and the error type
says so rather than implying otherwise.

## What it buys

### The caller's corpus

One copy of their tree, all eight runs inside it, flipping only the `[patch]` path between
a `git worktree` at the base commit and this one — both halves of each pair on the same
day, as `HANDOVER.md` insists, because that tree moves under us.

| lane, scale | base | with the bound |
|---|---|---|
| CPU, scale 1 | 931 agree / 23 differ / 2 refused / 18 not comparable | **931 / 23 / 2 / 18** |
| CPU, scale 4 | 936 / 11 / 4 / 23 | **937** / 11 / **3** / 23 |
| GPU, scale 1 | 929 / 25 / 2 / 18 | **929 / 25 / 2 / 18** |
| GPU, scale 4 | 937 / 10 / 4 / 23 | **938** / 10 / **3** / 23 |

**No page line of 956 moves at scale 1, in either lane** — all 25 and 27 printed lines are
identical to the character. At scale 4 exactly what
`doc/notes-tiling-ceiling.md` predicted happens and nothing else:

| page | lane, scale | base | with the bound |
|---|---|---|---|
| `bug1703683_page2_reduced.pdf` | CPU 4 and GPU 4 | refused, `ScratchExhausted` | **agrees with the oracle** |
| `issue1905.pdf` | CPU 4 and GPU 4 | refused, naming only the wall | refused, **naming the sheet**: a 4 763 × 7 103 tile against a sheet at 14 289 × 15 117 holding 6 tiles and 213 115 672 texels |
| `inks.pdf` | GPU 4 only | mean 0.0394, worst 17.29, differing 0.0012, SSIM 0.99861 | the same to the digit, SSIM **0.99862** |

Every other differing page's mean, worst tile, differing fraction and SSIM is identical to
the last digit at both scales in both lanes. `inks.pdf`'s fifth decimal is the 1-of-255
`fill_mask` residual ADR 0049 recorded and priced: a tile is asked for a different
rectangle and `f32` addition is not associative. Its mean, its worst tile and its differing
fraction do not move at all, and it does not move on the CPU lane.

**And the refusal that stays is now its own evidence.** `issue1905.pdf`'s message reports
the sheet the patched crate had to be built to read in `doc/notes-tiling-ceiling.md` §1 —
6 tiles seated and 213 115 672 texels, against a table that was obtained with `eprintln!`s
and read exactly 213 115 672.

**Their `REFUSED` ratchet fails, loudly, and that is the result rather than a problem.**
The gate holds the scale-4 refusal list to equality and prints both:

```
assertion `left == right` failed: the pages quorra refuses at 4× have changed
  left: ["bug1721218_reduced.pdf", "issue18032.pdf", "issue1905.pdf"]
 right: ["bug1703683_page2_reduced.pdf", "bug1721218_reduced.pdf", "issue18032.pdf", "issue1905.pdf"]
```

**The caller must drop `bug1703683_page2_reduced.pdf` from that list** when they take the
bump.

### The archetypes, and a fixture defect this made visible

`tests/archetypes.rs` gained a tenth column, `coverage.texels`, and **three rows moved** —
two of them because the change made a fixture defect visible that had been there since the
file was written:

| archetype | before | after |
|---|---|---|
| dense text | 40 tiles, 2 residue regions | **0 tiles, 0 regions** |
| artwork | 600 tiles, 185 regions, 0 residue tiles | **8 tiles, 2 regions, 6 residue tiles**, 12 284 texels |
| drawing | 6 tiles | 6 tiles, **245 texels** |

**Neither fixture's curve clips overlap the marks they clip.** `define_clips` places a
clip curve of about `side` across on a grid of step `side × 6`, while `emit` places its
marks on a grid of step `side`; counting the two boxes from the generator's own arithmetic
gives **0 of 40** for dense text and **8 of 600** for artwork, which is exactly the tile
count each now reports. Until a clipped mark's tile was bounded by its chain, those 632
commands rasterised a tile each and multiplied it by zero, so the row read as though the
residue lane were being exercised.

This is `HANDOVER.md`'s "a determinism fixture that does not overlap is not a determinism
fixture" in a second place, and it has two consequences that are stated here rather than
quietly absorbed:

- **the archetype signature no longer gates the residue lane at all**, and
  `tests/tiling_ceiling.rs` holds that property instead — 64 marks under clips that do
  overlap them, with `tiles == 64` asserted in both legs so the gate cannot pass by drawing
  nothing;
- **ADR 0049's artwork measurement was taken on this page.** `examples/residue_clip.rs`
  copies the archetype, so its 37.78 → 28.89 ms of geometry was mostly the removal of
  repeated rasterisation of tiles that drew nothing. The saving was real and the mechanism
  is unchanged; what the fixture demonstrated is narrower than the row implied. A fixture
  round is owed, and it is the first thing anybody measuring the residue lane must do.

## What it costs

- **A tile may be larger than the chain's exact bounds**, because the box is the control
  hull rather than the flattened outline. That is the price of not flattening, and it is
  bounded by the hull's slack over the curve.
- **Two boxes on one type.** `ResolvedClip` now carries `rect` and `residue_bounds` and a
  reader has to know which is asked for what. The alternative — intersecting the residue
  box into `rect` — is one field and one rule, and it was refused because `rect` is the
  shader's clip rectangle *and* `clip_distinct_regions`' key: folding a conservative
  curve-derived box into it changes what §6.4's instrument counts, which is a decision
  about a caller-visible number rather than about tiling.
- **`CoverageSheet::tiles` repeats `Counters::tiles`** on a drawn frame. One number with
  two names is a real cost and it is taken deliberately: the alternative is a refusal that
  cannot say how many tiles the page placed, and the pairing between the drawn and the
  refused state is the whole point of the type.
- **`issue1905.pdf` gets nothing**, which is the honest half. Its marks *are* the page:
  seven fills wider than the page under a rectangular clip that already bounds them,
  1 339 315 879 texels, no residue clip anywhere. Nothing on the tiling side draws that
  inside a 256 MiB budget. Before spending a round on it, ask the caller whether it refuses
  in the product or only in the gate — the frame that refuses is a whole page at 4× in one
  target, and a viewer's viewport is its window.
- **The pixels of a clipped mark can move by at most 1 of 255**, by the same mechanism
  ADR 0049 measured and for the same reason: `fill_mask` is asked for a different rectangle
  and `f32` addition is not associative. `a_tile_is_the_crop_of_the_region_that_contains_it`
  bounds it at 1 of 255 over 2 863 228 pixels and the corpus is what says whether any page
  notices.

## Revisit when

- **A page is found whose clip is as large as its marks and which still overflows the
  sheet.** This bound has nothing to give there; the levers left are the ones
  `doc/notes-tiling-ceiling.md` §4 priced and declined — a pane cut, a second sheet, a
  tighter packer — and it declined all three with the observation that the packer is
  already at 98.6 % and 97.4 % occupancy when each page refuses.
- **`commands_culled` is wanted to include marks a residue chain admits nothing of.** The
  bound already drops them from the sheet; what is missing is the count, and moving it is a
  change to a caller-visible number.
