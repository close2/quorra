# The tiling ceiling — what was measured, what it cost, what was found and not done

Round notes for `HANDOVER.md`'s item 2, written 2026-08-15 against this worktree. **No
`src/` file was changed by this round**; the decision it points at belongs in an ADR the
owner or a later round writes, and this file is the measurement behind it.

**The short version.** Two pages of the caller's corpus refuse with `ScratchExhausted`,
and it is not one problem but two.

- `bug1703683_page2_reduced.pdf` refuses because **a residue clip does not bound the tile
  its mark asks for**. 31 of its 34 coverage tiles are the whole page — 2 448 × 3 168 at
  4× — while the clip chains that admit them are as small as eleven pixels in all. The
  page asks for **1 008 561 911 texels of coverage where its chains admit 2 297 897** — a
  factor of **439**, and the sheet's 16 384-row ceiling is only the limit that fires
  first.
- `issue1905.pdf` refuses because **its marks really are the page**: seven fills, each
  larger than the page, under a rectangular clip that already bounds them to it. There is
  nothing to reclaim; the page asks for **1 339 315 879 texels**, five times the frame
  budget.

Neither page can be drawn by anything done to the sheet — a second sheet, a tighter
packer, a cut into panes all move the refusal from `ScratchExhausted` to
`FrameBudgetExceeded` and change nothing a caller can see. The packer is already at
**98.6 % and 97.4 % occupancy** at the moment each refuses.

**One candidate does draw one of them**, and it was run against the oracle rather than
argued: bounding a clipped mark's tile by its residue chain's own device box takes
`bug1703683_page2_reduced.pdf` from **1 008 561 911 to 2 511 363 texels** at 4× and from
refused to **agreeing with the CPU oracle**, with every other page line in the corpus
identical to the character at both scales. It is one hash lookup on a page that has a
residue clip and nothing at all on a page that has not.

**And the refusal itself is not diagnosable.** `ScratchExhausted { limit }` names the
adapter's wall and nothing about the frame that hit it, and a refused frame has no
`Frame`, so no `Counters`. Every number in this file was read from a throwaway
instrumented copy of the crate, because no combination of the public API can produce one
of them.

---

## 1. The shape of the refusal, exactly

The caller's corpus, one copy taken 2026-08-15 under `/home/AI/tiling-corpus/viewer`,
`PDFVIEWER_QUORRA_COVERAGE=cpu`, RADV. The numbers below come from `eprintln!`s in a
copy of this crate under `/home/AI/tiling-corpus/quorra-probe` — `ScratchPacker::reserve`
and `finish`, `Encoder::coverage_tile`, `Encoder::residue_intersection` and
`commit_sheet` — patched into that viewer copy. **Nothing here was committed and the copy
is scratch.** A refusal is arithmetic (`HANDOVER.md`), so these are exact and
machine-independent; only the adapter's 16 384 is a property of this machine.

### At which magnification it bites

| page | scale 1 | scale 2 | scale 4 |
|---|---|---|---|
| `bug1703683_page2_reduced.pdf` | draws, **agrees** | **refused** | **refused** |
| `issue1905.pdf` | draws, **agrees** | **refused** | **refused** |

`HANDOVER.md` says "at 4×". It is **at 2×**, which is an ordinary zoom step rather than a
corner of the corpus gate, and that is the first correction this round makes.

### `bug1703683_page2_reduced.pdf` at 4× — target 2 448 × 3 168

| | |
|---|---:|
| coverage tiles asked of the packer | **34** (33 seated, the 34th refused) |
| of which 2 448 × 3 168 — the whole page | **31** |
| the other three | 146 × 115, 146 × 115, 109 × 92 |
| tiles that came from `coverage_tile` | 33, **every one under a residue clip** |
| distinct residue chains among them | 33, one link each |
| of those, chains whose clip **rectangle is open** (±1e9) | **31** |
| texels seated when it refused | 232 701 528 |
| the sheet those sit in (widest cursor × `next_y`) | 14 688 × 16 070 = 236 036 160 |
| **occupancy at the refusal** | **98.6 %** |
| shelves | 7: heights 115, 115, 3 168, 3 168, 3 168, 3 168, 3 168 |
| the tile that did not fit | **2 448 × 3 168** |
| **which axis** | **height**: `next_y` 16 070 + 3 168 = **19 238 > 16 384**, over by 2 854 |
| the other axis | widest shelf cursor 14 688 of 16 384 — **89.6 %, never binding** |
| texels charged including the refused tile | 240 456 792 of 268 435 456 — 89.6 % of the budget |
| `Counters` | **none: the frame is refused, so no `Frame` exists** |

The seven shelves are the whole story of the ceiling: the sheet's height is the sum of
the shelves it opened, five of them 3 168 rows because a shelf fills in width before the
sheet gets tall (six page-wide tiles reach a cursor of 14 688 and the seventh opens
another shelf).

**What the page actually wants.** With the packer made to wrap instead of refuse and the
budget check disabled — so the walk finishes — the same frame asks for **149 tiles and
1 008 561 911 texels**: 130 of them page-sized. That is **3.8 × the frame budget**. The
16 384 ceiling is not this page's binding limit; it is the limit that fires first.

**And what its clips admit.** Those 149 tiles are asked for under **141 residue chains**,
130 of which ask for a tile over a million texels. Their chains' own device boxes:

| | |
|---|---:|
| smallest chain box | **11 × 1** = 11 texels |
| median chain box (of all 141) | **99 texels** |
| largest chain box | 1 188 × 1 168 |
| **sum of the chain boxes** | **2 297 897 texels** |
| **sum of the tiles they asked for** | **1 008 348 445 texels** |
| **ratio** | **439 ×** |

Forty-two of the 130 have a chain box eleven pixels wide and one or two tall, and each
is charged 7 755 264. Every texel outside those boxes is rasterised, multiplied by a clip
coverage of zero, shelf-packed, uploaded and sampled to no effect.

### `issue1905.pdf` at 4× — target 4 763 wide

| | |
|---|---:|
| coverage tiles asked | **7** (6 seated, the 7th refused) |
| their sizes | 4 763 × {7 710, 7 609, 7 509, 7 407, 7 305, 7 204} |
| residue clips | **none anywhere on the page** |
| what bounds them | a *rectangular* clip, (113.2, 114.0)–(4 875.2, 7 823.6) — the page |
| the marks themselves | fills of about 7 608 × 7 710 device bounds — **wider than the page** |
| texels seated when it refused | 213 115 672 |
| the sheet those sit in | 14 289 × 15 319 = 218 893 191 |
| **occupancy at the refusal** | **97.4 %** |
| shelves | 2: heights 7 710 and 7 609 |
| the tile that did not fit | **4 763 × 7 103** |
| **which axis** | **height**: 15 319 + 7 103 = **22 422 > 16 384**, over by 6 038 |
| the other axis | widest cursor 14 289 of 16 384 — **87.2 %** |
| texels charged including the refused tile | 246 947 261 of 268 435 456 — 92.0 % |
| `Counters` | **none** |
| **what the page wants in total** | **695 tiles, 1 339 315 879 texels — 5.0 × the budget** |

With the sheet unbounded this page is refused anyway, by a different budget, at
**452 120 912 bytes**. Its tiles take the *deferred sheet-job* path
(`encode/parallel/commit.rs`'s `commit_sheet`), not `coverage_tile`, which is why it
prints no residue line at all: `deferrable_bounds` sends every unclipped or
rectangularly-clipped CPU-lane mark there.

**These two pages have nothing in common except the shelf that refuses them.** One
placed 232 701 528 texels of coverage that its own clips say is 491 541; the other placed
213 115 672 that are exactly what the page asks for. `HANDOVER.md`'s single sentence —
"a clipped shape becomes one coverage tile of its own device bounds" — is right about the
first and does not describe the second.

### For reference, at scale 1, where both draw

| page | tiles | texels | shelves | sheet height |
|---|---:|---:|---:|---:|
| `bug1703683_page2_reduced.pdf` | 149 | 63 036 156 | 17 | 6 630 |
| `issue1905.pdf` | 1 293 | 77 076 597 | 18 | 10 983 |

`issue1905`'s 1 293 tiles at scale 1 are 37 tiles over a megapixel carrying **84.9 %** of
the sheet and 1 256 small ones carrying the rest.

---

## 2. Where a tile's size comes from, traced

Five places, and only one of them is a decision:

- **`encode.rs::coverage_tile`** — `shape bounds ∩ resolved.rect ∩ target`, floored and
  ceiled to whole pixels. It charges that box, calls `raster::fill_mask` over exactly it,
  and only *then* asks `residue_intersection` for the clip coverage to multiply in. The
  residue arrives after the size is already fixed.
- **`encode.rs::visible_tile`** — the same arithmetic without rasterising, for the GPU
  lane; `commit.rs::tile_bound` uses it to price a queued job.
- **`encode/parallel/commit.rs::deferrable_bounds`** — `clip ∩ target` for a deferred
  sheet job, and its own doc comment says it is "the same bound `coverage_tile` computes
  in place". This is `issue1905`'s path.
- **`encode/clips.rs::ClipResolver::resolve`** — **this is the decision.** A link whose
  outline has a `rect_hint` under an axis-preserving transform is intersected into
  `current.rect`; anything else becomes a `ResidueLink` and `rect` is carried through
  *unchanged*:

  ```rust
  None => ResolvedClip {
      rect: current.rect,                    // ← untouched
      residues: Some(Arc::new(ResidueLink { clip: link, parent: current.residues.clone() })),
  },
  ```

  That is where a full-page shape under an 18 × 19-pixel curved clip gets a full-page
  tile. It is deliberate for what it does — a residue link is not a rectangle and cannot
  *replace* the clip rectangle — but a residue link does have a bounding box, and nothing
  intersects it into anything.
- **`encode/clips.rs::chain_region`** — that box, already computed, already documented
  with the reason it is exact ("outside any one link's own bounds a closed path winds
  nothing"). It is computed **only** inside `residue_intersection`'s `Undecided` branch
  and **only** to price ADR 0049's region cache (`encode/residue.rs::admit`). It never
  reaches the tile.
- **`encode/scratch.rs::ScratchPacker::reserve`** — the ceiling. A tile seats on an
  existing shelf when `shelf_h ∈ [h, 2h]` and `cursor + w ≤ √(2 × area placed)`; otherwise
  it opens a shelf at `next_y`; otherwise `None`, which becomes `ScratchExhausted`. So
  **the sheet's height is the sum of the shelves it opened**, and `encode.rs` sizes the
  packer `ScratchPacker::new(max_dimension, max_dimension)` from the adapter's
  `max_texture_dimension_2d`.

**What a cut would have to preserve.** A coverage tile is not only bytes on a sheet:

- its `left`/`top` is the destination of the quad instance and the origin of its window
  into the sheet (`push_scratch_quad` → `push_quad_instance`), so pieces of a cut tile
  must **partition** the device rectangle — a device pixel drawn by two pieces is
  composited twice, which for anything but an opaque `SrcOver` is a different colour;
- each piece needs its own reservation and its own instance, so `Counters::tiles` — a
  caller-visible number and a row of `tests/archetypes.rs`'s signature — changes meaning;
- on the GPU lane a pane already redraws its tiles' triangles (ADR 0028 §3), so cutting
  one tile into *k* pieces costs *k* × that mark's vertices.

---

## 3. The invariant any cutting scheme must satisfy

`HANDOVER.md`'s trap states it: **a tile is not a window on a wider region unless the
rasteriser cuts at its border.** `raster::deposit_slab` once clamped the endpoints of a
slab piece that left the region and interpolated between them; the row's total winding
survived, so every column past the crossing read correctly and no test could see it,
while the columns *at* the border took the height the piece spent outside — 2 684 pixels
of 2 863 228, worst by 185 of 255. ADR 0049 replaced the clamp with a cut at the border.

The probe is `raster.rs`'s `a_tile_is_the_crop_of_the_region_that_contains_it`: forty
closed curves, each rasterised over its own bounds and then over twenty tiles cut out of
it at offsets that hang off every side, every pixel compared at the same *device* pixel.
It reads **31 pixels of 2 863 228, every one by 1 of 255**, and that residual is `f32`
non-associativity rather than geometry.

**What that means for a scheme that asks the rasteriser for a smaller rectangle** — which
is what both candidates below do — is that the probe *is* the question, and it already
passes: each piece is `fill_mask` over its own rectangle, and the border cut is inside
`deposit_slab` where every such call goes. On paper and in the tree, a cut-into-panes
scheme and a shrink-the-tile scheme are both bounded at 1 of 255 per pixel by that probe,
with no further argument needed.

**What the probe does *not* cover, and each candidate must answer separately:**

1. **A scheme that rasterises once and slices the bytes afterwards** is a different thing
   and would need the probe re-run against its slicing, not against `fill_mask`. Neither
   candidate below does that.
2. **A shrink needs the removed pixels to be zero**, which the probe says nothing about.
   For the residue box this follows from ISO 32000-2 §8.5.4 as ADR 0030 reads it — the
   chain is the `min` over its links, and outside a closed link's own bounds it winds
   nothing — and it is `chain_region`'s own stated reason for being an intersection. It
   was not left as an argument: §4 checks it in the pixels, on 956 corpus pages.
3. **Pieces of a cut must partition, not overlap**, which is the compositor's question
   rather than the rasteriser's and has no probe today.

The 1-of-255 is a real cost and belongs in the ADR: it is the same class of movement
ADR 0049 recorded and priced, and the corpus is what says whether any page notices.

---

## 4. The shortlist, each with its cost

### 1. A residue chain contributes its box to the tile — **recommended**

Before `coverage_tile` charges, intersect the tile with the chain's device box — the box
`chain_region` already computes — memoised per chain key beside `ResidueRegions`. A chain
whose box is empty draws nothing at all.

| | |
|---|---|
| `Counters::tiles` | **unchanged** on every archetype and on both pages — it moves tile *area*, and there is no counter for that (see §5) |
| per-frame upload bytes | `bug1703683` at 4×: **1 008 561 911 → 2 511 363 texels (402 ×)**; at scale 1: **63 036 156 → 160 244 (393 ×)**; `issue1905`: **unchanged**; artwork archetype: **single-digit per cent**, estimated from its 185 first asks |
| a second pass over the commands | **no** — the box comes from the flatten the chain's first ask already does |
| what the common case pays | **nothing**. `resolved.residues.is_none()` is already the first question on that path (`deferrable_bounds`); a page with no residue clip never computes a box |
| fidelity | the mark's `fill_mask` is asked for a sub-rectangle — the probe's own question, ≤ 1 of 255 |

**Measured on the caller's corpus**, one copy, both runs the same hour, only this
candidate flipped between them (`cpu` lane):

| lane, scale | base | with the box |
|---|---|---|
| scale 1 | 931 agree / 23 differ / 2 refused / 18 not comparable | **931 / 23 / 2 / 18** |
| scale 4 | 936 / 11 / **4** / 23 | **937** / 11 / **3** / 23 |

At scale 1 **all 25 differing-and-refused page lines are identical to the character**. At
scale 4 all 11 differing lines are identical to the character and
`bug1703683_page2_reduced.pdf` moves from **refused to agreeing with the oracle**. Nothing
moved away from the oracle at either scale.

*(This copy's own scale-4 base reads 936 / 11 / 4 / 23 where `PLAN.md` records
936 / 10 / 5 / 23. Their tree moved under us, which is exactly why `HANDOVER.md` insists
the baseline be run in the same copy on the same day. The comparison above is that.)*

**What it does not fix**: `issue1905`, and any page whose clip is as large as its marks.
The artwork archetype is the latter — its 185 chains are tight around the 600 marks under
them (their boxes are 95 % of the tiles asked for), which is exactly the shape ADR 0049's
admission rule was written for, and this candidate is worth ~5 % there. **The two shapes
are complementary and both are needed**: ADR 0049 removes repeated rasterisation of one
region; this removes coverage that no region admits.

### 2. A tile is cut into panes of bounded height (ADR 0028's mechanism, on the sheet)

| | |
|---|---|
| `Counters::tiles` | **grows by the number of pieces** — a caller-visible number changes meaning and every archetype row with a tile in it moves |
| per-frame upload bytes | **unchanged.** The sheet packs tighter — its height stops being a sum of tile heights and becomes about `√(2 × area)` — but every texel is still rasterised and uploaded |
| a second pass | **no** for a fixed pane height; **yes** if the height is chosen from what the frame will hold |
| what the common case pays | a comparison per tile, plus a constant nobody measured — the shape ADR 0027 had to delete twice |
| does it draw the two pages? | **No.** They want 1.008 GB and 1.34 GB against 268 435 456 bytes: with the height ceiling gone both refuse on bytes instead |

Where it *would* be the answer is a page between about 134 M and 268 M texels of
coverage, where `√(2A)` crosses 16 384 before the bytes do. **Neither of the two pages
that reach this ceiling is in that band** — they overshoot the budget by 3.8 × and 5.0 ×.
No claim is made about the rest of the corpus; nobody has measured their totals.

### 3. A second sheet — already refused, and this round adds a number

`HANDOVER.md` records it as measured and refused. What this round adds is that the refusal
now covers both of the pages that actually hit the ceiling and not only
`bug1721218_reduced`: **1.008 GB and 1.34 GB against a 268 435 456-byte budget** means a
second sheet does not draw either of them at 4× either. It moves the refusal, at the cost
of a second texture and a sheet index in every batch.

### 4. Sorting tiles by what packs tightest (ADR 0034), or a residue region grown to the union of the tiles that ask (ADR 0049)

Both need something known before the walk that only a second pass provides, and both are
recorded as declined. This round adds the number that says a better packer has nothing to
find here: **the sheet is 98.6 % and 97.4 % occupied at the moment each page refuses**, so
perfect packing would buy 1.4 % and 2.6 % against overshoots of 17 % and 37 % of the
ceiling.

### 5. Refuse better — cheap, independent of every option above, and owed

`RenderError::ScratchExhausted { limit }` names the adapter's wall and nothing about the
frame that hit it: not the sheet's extent, not the tile that did not fit, not how much was
placed. A refused frame has no `Frame` and so no `Counters`, so a caller cannot even say
how many tiles the page had. Principle 6 asks a failure to name **what overflowed**; this
one names the wall.

Three numbers make both pages diagnosable without a debugger — the sheet's extent when it
failed, the tile that did not fit, and the texels placed — and they cost an error
variant's fields. **The whole of §1 of this file had to be obtained by patching the
crate**, which is the argument.

There is also no `Scene::cost()` answer here and there cannot be one: the sheet's height
is a function of the *viewport*, which a `Scene` does not have. So this is the one budget
principle 6's "discoverable before the frame" does not reach, and saying so plainly is
better than a `limits()` field that implies otherwise.

---

## 5. Recommendation

**Take candidate 1, and take 5 with it. Leave 2, 3 and 4.**

Candidate 1 is the only one measured to draw a page that is refused today, it is the only
one that reduces what a frame *asks for* rather than how it is packed, it costs a page
with no residue clip nothing, and the corpus says it moves no pixel of the rest of the
corpus. Candidate 5 is a day's work that makes the next round of this seam start from
numbers instead of from a patched crate.

What that leaves undone, stated so nobody mistakes it for solved:

- **`issue1905.pdf` at 4× stays refused, and correctly.** Seven marks that each cover a
  4 763 × 7 710 page, drawn whole into one target, are 1.34 GB of coverage. Nothing on the
  tiling side draws that within a 256 MiB budget, and drawing it outside one would be
  §6.2's failure wearing a success's name.
- **But check the shape with the caller before anyone spends a round on it.** The frame
  that refuses is *a whole page at 4× in a single target*, which is the corpus harness's
  shape. A viewer showing that document zoomed to 4× hands us a viewport the size of its
  window, and every one of those seven tiles is `shape ∩ clip ∩ target` — bounded by the
  window, not by the page. Whether `issue1905` refuses in the product or only in the gate
  is a question for `QUORRA_FEEDBACK.md`, and it decides whether this half of the seam is
  worth any work at all.
- **`Counters` has no field for what a frame's coverage costs** — not the sheet's extent,
  not its texels. `tiles` is a count, so candidate 1 is invisible to
  `tests/archetypes.rs`'s signature: a 402 × reduction in coverage bytes would not move a
  single row of it. This is the same gap `doc/notes-encode-threads.md` §5 found from the
  other side ("`Counters` has no field for segments per tile"), and it is now two rounds
  in a row that could not gate what they measured. Reported rather than added: `Counters`
  is caller-visible API and belongs to a bump.
- **The residue box is not free of a clause question**, and the ADR should say so rather
  than inherit this file's confidence. It rests on ADR 0030's reading of §8.5.4 — the
  chain is one region, arrived at by intersection, so outside any link's own bounds it
  admits nothing. That is the same sentence `chain_region` already relies on to price the
  region cache, so the candidate adds no new reading; it acts on one already in the tree.

## 6. What is in the tree from this round

`crates/quorra-gpu/tests/tiling_ceiling.rs`, two probes, both verified able to fail:

- `a_residue_clip_does_not_bound_the_tile_its_mark_asks_for` — sixty-four page-sized marks
  under twelve-pixel clips, once curved and once rectangular, against a budget with room
  for four pages of coverage. The rectangular leg draws and the curved leg is refused,
  which is §1's finding in the public API. **Verified able to fail by building candidate 1
  into the scratch copy and running this file there: it fails, as it should.**
- `a_frame_is_refused_for_the_sheets_height_with_its_bytes_untouched` — eight tiles of
  increasing height, 175 000 texels in all, refused for the sheet's height at 0.07 % of
  the frame budget. A **stand-in for the mechanism and not for the magnitude**: it opens a
  shelf per tile because each is taller than the last, where the corpus's pages open one
  because a shelf fills in width first. What both share, and what it pins, is that the
  ceiling is a sum of shelf heights and that the two budgets are reached independently.
  Verified able to fail by asking for four tiles instead of eight.

Neither needs a corpus, and both run in 0.2 s on `llvmpipe`.
