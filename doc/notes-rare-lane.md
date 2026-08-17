# What the rare lane's coverage is worth on the device

Status: **record** — a measurement, its instrument, its date, and how to redo it. The
decision it supports is ADR 0064.

`Options::coverage` selects who rasterises coverage: the CPU scanline rasteriser
(ADR 0008) or the device (ADR 0016). **It does not reach a rare paint at all.**
`Encoder::take_gpu_lane` is consulted in exactly two places, `encode/fill.rs`'s
`fill_solid` and `encode/coverage.rs`'s `push_coverage_styled`, and both are the *solid*
arm; a shading, an image, a mesh or a §7.10.5 function paint reaches the scratch sheet
through `encode/rare.rs`'s `push_rare_coverage`, which calls `coverage_tile` directly and
never asks.

That was found on 2026-08-16 and half-closed — the public rustdoc on `Coverage` was made
to *say* it — and left as "an open question with a measurement in front of it". §11.2's
census (`doc/notes-census.md`, 2026-08-17) made it worth asking, because it found that the
path lane inverts by work: a tenth of a page's marks, **at least 66 % of the coverage the
frame rasterises**. This is that measurement.

**The answer: of the coverage the sheet rasterises, the part that is a rare paint's *and*
is eligible to move is 0.110 % at scale 1 and 0.629 % at 4×.** Left, deliberately, and
ADR 0064 writes down the cost.

---

## 1. What was measured, and where

Every coverage tile a frame seats on the scratch sheet, split by the call site that seated
it, over the caller's corpus. The split is a partition, and it reconciles: the six buckets
sum to `Counters::coverage.texels` **exactly**, on every one of 3 897 frames across four
configurations.

- **Instrument:** a throwaway `Probe` on `Encoder`, incremented at the six sites that seat
  a tile, printed from `encode/encoded.rs`'s `finish` under `QUORRA_PROBE`. §6 has it and
  why it is not in the tree.
- **Corpus:** the caller's gate corpus — 974 documents' first pages, `doc/pdf.js/test/pdfs`
  in `/home/cl/projects/pdf-viewer` at **`22ab57d4`**, copied on **2026-08-17**. *Their
  tree moves under us: a count in an older document is not a baseline* (`HANDOVER.md`).
- **quorra:** `main` at **`de1c013`** plus the probe. Every row was first taken at
  `1cd74c9` and **re-taken** on `de1c013` after ADR 0063 landed; §7 says what moved.
- **Adapter:** `AMD Radeon 890M Graphics (RADV STRIX1)`, one encode thread, quantum off
  (the corpus gate's own configuration).
- **Scales 1 and 4, both lanes.** A census without a scale is not a census, and the lane
  matters here because the *denominator* changes completely between them.

### The six buckets

| bucket | where it is seated | what it is |
|---|---|---|
| **solid** | `push_coverage_styled`'s CPU branch, `commit_sheet`, `commit_glyph`'s atlas overflow | every solid-painted tile the CPU rasterised |
| **device** | `push_gpu_tile` | ADR 0016's lane, empty under `Coverage::Cpu` |
| **rare, residue** | `push_rare_coverage` under a residue clip | ineligible by the rule already on the `Gpu` variant |
| **rare, eligible** | `push_rare_coverage`, no residue, and ADR 0026's cost comparison passes | **the population this round is about** |
| **rare, dear** | `push_rare_coverage`, no residue, triangles cost more than the tile | ADR 0026 keeps these on the CPU lane whatever the setting says |
| **image residue** | `encode_image`'s residue product | an image without a residue clip rasterises nothing |
| **soft mask** | `encode/layer.rs`'s mask tiles | consumes coverage, does not choose a lane |

"Eligible" is asked in ADR 0026's own units and with its own inputs: the shape's device
tile (`tile_side` of the polylines' bounds, which is what `push_coverage_styled` hands
`take_gpu_lane`) against `triangles × 3 × WindingVertex::STRIDE`, with
`CacheProspect::TooLarge` for the cache condition, because a rare paint's coverage is never
offered to the atlas.

## 2. The numbers

**`Coverage::Cpu`** — the caller's default, and what `viewer-ui` draws every page with:

| bucket | scale 1: texels | share | marks | scale 4: texels | share | marks |
|---|---:|---:|---:|---:|---:|---:|
| solid | 116 363 007 | 98.569 % | 82 958 | 360 325 360 | 93.144 % | 101 717 |
| rare, **residue** | 954 449 | 0.808 % | 350 | 15 111 021 | 3.906 % | 350 |
| rare, **eligible** | **129 455** | **0.110 %** | **28** | **2 433 115** | **0.629 %** | **88** |
| rare, **dear** | 73 646 | 0.062 % | 181 | 654 132 | 0.169 % | 121 |
| image residue | 301 507 | 0.255 % | 10 | 4 660 145 | 1.205 % | 9 |
| soft mask | 229 870 | 0.195 % | 3 | 3 665 062 | 0.947 % | 3 |
| **sheet** | **118 051 934** | | 83 530 tiles | **386 848 835** | | 102 288 tiles |

**`Coverage::Gpu`** — the setting the question is actually about, because it is the one a
caller sets and does not get:

| bucket | scale 1: texels | share | scale 4: texels | share |
|---|---:|---:|---:|---:|
| **device** | 118 814 319 | 93.903 % | 353 708 595 | 85.989 % |
| solid, still CPU | 6 026 133 | 4.763 % | 31 107 548 | 7.562 % |
| rare, residue | 954 449 | 0.754 % | 15 111 021 | 3.674 % |
| rare, **eligible** | **129 455** | **0.102 %** | **2 433 115** | **0.592 %** |
| rare, dear | 73 646 | 0.058 % | 654 132 | 0.159 % |
| image residue | 301 507 | 0.238 % | 4 660 145 | 1.133 % |
| soft mask | 229 870 | 0.182 % | 3 665 062 | 0.891 % |
| **sheet** | 126 529 379 | | 411 339 618 | |

**The sharper framing, and it is the one to quote.** Under `Coverage::Gpu` the CPU still
rasterises 7 715 060 texels at scale 1 and 57 631 023 at 4×. The eligible rare paints are
**1.68 %** and **4.22 %** of that remainder. So a caller who asks for the device lane and
does not get it for rare paints is being denied between one part in sixty and one part in
twenty-four of the work that is left on the processor — and the whole of the rest of that
remainder is residue clips, soft masks and the marks ADR 0026 chose to keep.

Two figures that do not fit in the tables:

- **All rare-painted coverage is 0.98 % of the sheet at scale 1 and 4.70 % at 4×.** Four
  fifths of it is under a residue clip, which the device lane cannot draw *at all* until
  something on the device multiplies a residue — ADR 0016's own recorded gap, not this
  round's.
- **559 marks**, at both scales: 350 residue-clipped, 209 not. The 209 do not change with
  the scale; what changes is how ADR 0026 sorts them, 28 eligible at 1× and 88 at 4×,
  because a tile's area grows sixteenfold while its triangle count does not move.

## 3. How concentrated it is, and why that settles it

Not a tail; not even a distribution.

- **25 pages of 954 draw a rare-painted coverage tile at all.** Nine draw an eligible one
  at scale 1, fourteen at 4×.
- The largest eligible page at 4× is `pattern_text_embedded_font.pdf` at 694 697 texels —
  19.5 % of that page's own sheet, and **0.18 %** of the corpus's.
- Six pages at 4× are 100 % eligible by their own sheet (`bug1019475_1.pdf`,
  `issue13325_reduced.pdf`, `issue17065.pdf`, `issue6769.pdf`, `issue6769_no_matrix.pdf`,
  `issue9243.pdf`) — and their whole sheets are 73 904 to 508 110 texels. A page whose
  entire coverage bill is a third of a megapixel is not a page any lane choice rescues.

For comparison, on the same corpus and the same instrument, five pages carry 87 % of the
scale-1 sheet and `issue1905.pdf` alone is 68 % of it — 78 620 979 texels, **607× the whole
eligible population at that scale**, and it has no rare paint on it anywhere.

## 4. What the eligible marks *are*, and why that reverses the sign

The count and the coverage point in opposite directions, and it is the count that decides.

Per-mark, at 4×, over the fourteen pages with an eligible mark:

| page | eligible marks | mean tile |
|---|---:|---:|
| `ShowText-ShadingPattern.pdf` | 29 | 3 639 texels (≈ 60 × 60 device, **15 × 15 at the page's own scale**) |
| `bug1019475_1.pdf` | 12 | 9 290 |
| `issue8111.pdf` | 7 | 7 175 |
| `issue5804.pdf` | 6 | 1 418 |
| `issue19360.pdf` | 2 | 4 200 |
| the other nine pages | 32 | 15 194 – 144 472 |

**56 of the 88 eligible marks are at or below ten thousand device texels** — under
100 × 100 device pixels, which at 4× is under 25 × 25 at the page's own scale: reading size
and just above it. They are 11.7 % of the eligible texels. The eligible *coverage* is a
handful of large pattern fills; the eligible *marks* are mostly glyphs.

`ShowText-ShadingPattern.pdf` is the case in one page. It draws 63 shading-painted glyphs
and nothing else. At scale 1 all 63 are **dear** — 15 332 texels for the page, 243 each,
about 15 × 16 — so ADR 0026's comparison keeps every one of them on the CPU lane. At 4× the
tiles grow sixteenfold, the triangle counts do not, and 29 of the 63 cross the criterion.
They are the same glyphs.

That is the finding this round did not go looking for: **the comparison that would decide
is missing its most protective condition for exactly this population.** In the solid arm
`take_gpu_lane` has four conditions, and the one that keeps reading-size text off the
sampled grid is the *cache* — `CacheProspect::worth_caching`, which answers "the atlas will
hold this and re-read it" for a glyph. A rare paint's coverage is never offered to the
atlas, so that condition is `TooLarge` by construction and only the tile-versus-triangles
comparison is left. And that comparison is a monotone function of the magnification: the
higher the zoom, the more of a page's shading-painted text it sends to the device. Which is
precisely where the caller switches to `Coverage::Gpu`.

## 5. Both sides of ADR 0026's comparison, priced

The comparison is two costs, so both are counted rather than one being argued.

**The device side.** The eligible population's triangles are **48 000 bytes at scale 1 and
253 440 at 4×** against 129 455 and 2 433 115 texels of coverage — the device is cheaper by
2.7× and 9.6× on ADR 0026's own arithmetic, which is exactly why these marks are in the
eligible bucket. So asked, the criterion *would* answer "the device", and it would not be
answering it wrongly by its own terms.

**The processor side.** 129 455 texels of exact-area scanline coverage at scale 1, spread
over 28 marks on nine pages of 954. `HANDOVER.md`'s seam (ADR 0052) forbids turning that
into a duration on this machine, and it does not need one. For scale: **701 of the 954 pages
seat no sheet tile at all**, the p90 page seats 19 548 texels, and `issue1905.pdf` alone
seats 607 times the whole eligible population. The saving is not a number this machine can
fail to measure; it is a number the corpus says is not there.

**And a third cost the comparison does not carry.** The two lanes charge different budgets.
`coverage_tile` calls `charge_tile(width, height)`, so a CPU tile is charged its texels
against `max_frame_bytes` before it is rasterised; `push_gpu_tile` charges nothing there and
the lane's cost arrives as `winding.device_bytes()` at `finish`. Moving a mark between the
lanes therefore moves which allocation refuses a frame. On a page of large shading fills
that is the winding texture at eight bytes a texel — the memory ADR 0026's last row is still
about — so "the device is cheaper" in triangle bytes is not "the frame is cheaper" in the
number principle 6 refuses frames on.

## 6. Is it an oversight or a design constraint? — an oversight, and here is the proof

The question was to be answered from `push_rare_coverage` rather than assumed. Read it, and
then run it.

**Read.** A rare paint needs its coverage as an R8 tile in the frame's sheet plus the tile's
origin: `QuadPlacement::coverage_origin`, which `shading.wgsl` and `function_lane.wgsl` turn
into `textureLoad(scratch_tex, coverage.xy + (p − dest.xy))`. The device lane's output is
already exactly that:

- `push_gpu_tile` takes its seat from `ScratchPacker::reserve`, which is **the same one door
  onto the sheet** `pack_scratch` goes through — `encode/scratch.rs`'s module comment states
  it: "one door … and two producers behind it".
- `device/staging.rs` renders the winding tiles into the frame's R8 scratch texture during
  the upload phase, **before any draw pass is recorded**, so a quad drawn later cannot tell
  which producer wrote its texels. That is ADR 0016's stated integration.
- The tile's extent comes from `visible_tile` in both branches, which `encode/coverage.rs`'s
  module comment keeps in one place on purpose.

So the only thing standing between `push_rare_coverage` and the device lane is that
`push_gpu_tile` does two things in one function — reserves and draws the triangles, *and*
emits the solid quad instance — and the rare lane needs the first half and builds its own op.

**Run.** That reading was verified by making the change, in about fifty lines, as the forced
defect for this round's gate: `coverage_placement` asks `take_gpu_lane`, reserves, appends
`append_polyline_triangles`, pushes the winding tile, and returns a `QuadPlacement` on that
seat. It compiles, it draws, and the frames it draws differ from the CPU lane's **only in
the antialiased edges** — 419 pixels on each of the two marks of
`tests/rare_lane_coverage.rs`, which is the 4 × 4 sample grid against exact area and nothing
else. Both function-coverage gates and the new shading gate fail under it, which is what a
gate is for. The patch was then reverted; it is not in the tree.

**So it is an oversight, not a constraint — and it is still not worth taking.** Those two
sentences are not in tension. The reason to leave it is the measurement above, and the
reason to say it is an oversight is that a future round must not re-derive the structural
question from scratch and must not be told it is impossible.

Two asymmetries a future round would have to answer, neither of them a blocker:

1. **A rare fill arrives already flattened.** `encode_fill` calls `raster::flatten` before
   `push_rare_coverage`, so the device lane would draw polylines, not the outline's
   quadratics — it would get ADR 0016's *rasterising* saving but not its scale-independence,
   which is the half that matters at zoom. Getting that too means deferring the flatten past
   the lane choice, which is a change to the fill arm rather than to the rare one. (A rare
   *stroke* has no such half: its expansion is polylines under any lane, exactly as
   `push_coverage_styled` already draws a solid one.)
2. **The budget seam of §5.** Which allocation a mark is charged to changes with its lane.

## 7. What the rebase onto `de1c013` moved: nothing

Every row was first taken at `1cd74c9` and re-taken after ADR 0063's atlas round landed,
rather than any of them being asserted unchanged (`doc/notes-census.md` §11's precedent).
The scale-1 `Coverage::Cpu` run's 1 939 probe and page lines are **character-identical**
across the two bases. ADR 0063 added `Counters::atlas_overflow_tiles` and `Limits::atlas_bytes`
and changed no policy, so this is what it should have shown; it is recorded because a
should-have is not a did.

The scale-4 runs end in the caller's own stale ratchet — `the pages quorra refuses at 4×
have changed`, missing `bug1703683_page2_reduced.pdf`, which ADR 0057 moved from refused to
drawn and which `HANDOVER.md` already lists as owed to the caller. It fires *after* every
page has been rendered, so all 965 probe lines are present. Nothing in this round caused it
and nothing in this round fixes it.

## 8. Method, so a later reader can redo it

Two pieces, neither of them in this tree.

1. **The harness is the caller's own corpus gate**, in a *scratch copy* of their tree per
   `HANDOVER.md`'s rsync recipe plus a `[patch]` block. Unlike `doc/notes-census.md` §6 it
   needs **no edit to their `QuorraRasterizer`**: the probe prints its own line to stderr,
   so nothing has to travel out through `Counters`. One three-line edit to their
   `tests/corpus.rs` prints `PAGE <name>` before each `quorra.rasterize`, which is only for
   attribution; the totals do not need it.

   ```
   QUORRA_PROBE=1 PDFVIEWER_QUORRA_SCALE=n PDFVIEWER_QUORRA_COVERAGE=cpu|gpu \
   CARGO_TARGET_DIR=<private> cargo test --release -p render-quorra --test corpus \
     -- --ignored --nocapture 2> out.txt
   ```

   27 s at scale 1, 280 s at scale 4 (the oracle runs too; `notes-census.md` §6's
   quorra-only harness is half that if only the counts are wanted).

   **The gate re-renders 29 to 37 pages** to dump artefacts for the ones that differ, so the
   frame count is above the page count and those pages' tiles are counted twice. It does not
   move any share by more than a thousandth, and the per-page tables above are taken from
   first occurrences.

2. **The probe**, a `Probe` struct on `Encoder` with six pairs of counters, incremented at
   the six seating sites and printed from `finish`. It is **not** in the tree, for
   `notes-census.md` §6's reason read for this quantity: a *reason* a tile was seated has to
   pick one of several true answers, and `Counters::coverage` (ADR 0057) already publishes
   the total that a public question would ask about. **No `Counters` field was added by this
   round**, and none is proposed: `coverage`, `lanes` and `atlas_overflow_tiles` already
   carry every number a caller has a question for, and a fourth would be the duplicate the
   census round refused.

## 9. What this measurement cannot see

- **First pages only**, one per document, which is the caller's own gate population. A
  document whose *later* pages are chart-heavy is invisible here, and shading-painted
  artwork is exactly the kind of content that sits past page one.
- **It is a count, not a clock.** What a lane costs per mark is a property of the processor
  and the adapter together (`HANDOVER.md`'s trap) and is `tests/lane_crossover.rs`'s subject.
  Every number here is exact and load cannot touch it, which is why the runs were taken at
  load average 10–20 without apology.
- **The eligible bucket is ADR 0026's criterion, not a claim that the device would be
  faster.** It says what the comparison would answer if it were asked. §5's third cost is
  not in it.
- **A mesh paint appears nowhere in the corpus's rare tiles**, so "shading, image, function"
  is what was actually measured of the four the claim names.
- **`Coverage::Gpu`'s own placement census (ADR 0029)** reads only under that setting and
  cannot affect the rare buckets, which are identical across all four runs to the texel —
  which is itself the evidence that the setting does not reach them.
