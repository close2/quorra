# A clipped page's measurements did not mean what they said

Round notes for 2026-08-17. One subject in two halves, and they interact: **half 1 changes
what `recording` means on a clipped page, and half 2 changes what the clipped page is.**
Each was done and measured separately so that every number below has one cause, and each
table says which half it belongs to.

**The short version.**

- **Half 1.** The residue multiply — the per-pixel product of a clip chain's coverage into
  a mark's tile — sat outside every span of the encode clock, so it was reported as
  `recording`. It is geometry: it *makes* the mark's coverage. One seam moves it, and
  ADR 0023 is amended with the rule that decides such cases and with what the seam costs.
- **Half 2.** `tests/archetypes.rs`'s two curve-clipped pages placed their clips on one
  grid and their marks on another, so **0 of 40** and **8 of 600** clipped commands had a
  mark that met the clip clipping it (found by ADR 0057, written down in
  `doc/notes-tiling-bound.md` §3). The clips are now cut around the marks they clip, in the
  fixture and in the four examples that copy it, and a gate fails if that stops being true.
- **Neither half draws anything differently.** No encode behaviour changed — the clock is a
  clock, and a test fixture is not the corpus — so no corpus run was owed and none was
  taken.
- **One thing fell out of it: `examples/retained.rs` has been failing its own signature
  gate since ADR 0057** and nothing noticed, because an example is not run by
  `cargo test`. Its copy of the dense-text row still said 40 tiles for a page that had
  stopped drawing any. The re-cut makes the page draw 40 again, so the number is right for
  the first time in two days rather than merely correct-looking.

---

## 1. Half 1 — what was mislabelled, and the fix

`Options::instrument_encode` reports three phases and ADR 0023 defines them: geometry is
flattening, stroke expansion and the scanline pass; staging is packing into the sheet and
the atlas; **`recording` is the remainder**, computed rather than measured.

`encode/coverage.rs::coverage_tile` opened a geometry span around `raster::fill_mask` and
closed it immediately after. The next statement multiplied the chain's residue into the
tile just rasterised — one multiply, one add and one divide per pixel — with no span open.
Everything else on that path was already spanned: the links' flatten, their `min`, and a
region's crop all sit inside `clock.geometry` spans in `encode/clips.rs`. So the residue
product was the **only** unattributed per-pixel work on the clipped path, and every
`recording` figure this project has published for a page with a curve clip on it was too
large by exactly it.

The fix is one seam: the loop is `residue_product`, a function of its own, inside its own
`clock.geometry` span.

**The line the amendment writes down**, because the phase definitions did not decide this
case: *geometry is the work that makes coverage; recording is the work that decides what
to do with it.* That is why the residue product moves and `raster::polyline_bounds` does
not — bounding a shape is the input to a decision, which is also why `HullMemo::bounds`
stays in `recording` where it is the largest single row of a path-heavy page's recording
(`doc/notes-recording-shares.md` §2.2). "It is per-pixel, therefore it is geometry" would
move that row too.

**What the seam costs.** Two `Instant::now()` reads — ~20 ns each — per clipped command
that rasterises a tile under a residue chain, and only while the switch is on: 1 200 reads
and ~24 µs on the artwork archetype's 600 clipped commands, against an encode of ~53 ms.
**0.05 %.** With the switch off, `EncodeClock::start` is a branch on a `bool` that returns
`None` and no clock is read at all. A page with no curve clip does not reach the line.

---

## 2. What half 1 moved — measured on the page as it stood

**Instructions, because this is a claim about *how much* and not about *how fast***
(`doc/HANDOVER.md`, and ADR 0052's seam). The harness is that file's "An encode, exactly":
a `#[cfg(test)]` module in `quorra-gpu` that builds a `ResourceStore` and an `AtlasStore`
directly, encodes once outside the collected region and once inside an
`#[inline(never)] steady_run`, built with `CARGO_PROFILE_RELEASE_DEBUG=1` into its **own**
`CARGO_TARGET_DIR`, run under `valgrind --tool=callgrind --collect-atstart=no
--toggle-collect='*steady_run*' --cache-sim=no`. Deleted with the round.

**What says it encoded the right page**, before any share is believed: its counter row is
`tests/archetypes.rs`'s artwork row to the digit, in both configurations —
`[684, 300, 300, 1, 8, 2, 6, 12 284]` for the page as it stood and
`[684, 300, 300, 1, 600, 66, 384, 3 542 360]` for the re-cut one.

*(One trap for whoever writes the next harness: the warm-up encode and the collected one
had identical bodies, the compiler folded them into one symbol, and the toggle then
collected **both** — visible only because `residue_product` was entered 1 200 times on a
page with 600 clipped commands. The warm-up now differs from the measured run on purpose.)*

| | artwork **as it stood** (half 1's page) | artwork **re-cut** (half 2's page) |
|---|---:|---:|
| one cold-atlas encode | 180 742 284 Ir | 757 574 442 Ir |
| `residue_product` | **16 490 Ir** | **4 683 942 Ir** |
| — of the encode | 0.01 % | **0.62 %** |
| — of `recording` before the seam moved | **0.06 %** | **13.7 %** |
| calls / tile pixels | 6 / 12 284 | 600 / 3 542 360 |

`recording` is not measured by callgrind either, so it is the same remainder computed in
instructions: encode less the inclusive cost of every function a geometry or staging span
wraps (`fill_mask`, `flatten`, `stroke_polylines`, the region crop, `residue_product`;
`ScratchPacker::pack` and `AtlasStore::insert`). On the re-cut page that is geometry
95.6 %, staging 0.5 %, recording 3.9 % — **and those are not the shares the clock reports**,
because staging is a memcpy of three and a half megabytes and instructions are the wrong
unit for it. That is the point of having both instruments rather than one.

**The finding this leaves for half 1 on its own: the published 56 % could not have been
re-measured on the tree it was published against.** ADR 0057 had already taken 592 of the
600 tiles away, so the residue product on the artwork page was 16 490 instructions —
0.06 % of that page's recording. The mislabel was real; the page that made it look large
had stopped existing.

### 2.1 And the wall clock, labelled as the weaker instrument

`examples/residue_clip.rs` on the re-cut page, llvmpipe, headless into a texture created
once, `instrument_encode` on, minima of twenty steady frames with the first reported apart,
three rounds alternating between the two builds — the A/B recipe `doc/HANDOVER.md`
prescribes for this example. Milliseconds; the load average is beside each row because this
machine is somebody's desktop and it went from 380 to 4 while this round ran.

**A** is the span present, **B** the span absent — the tree as it was before this round —
and the three parts are read off the **same frame**, the one whose encode was fastest,
because the minimum of a *remainder* taken across twenty frames is not the remainder of the
minimum and picks whichever frame the scheduler was kindest to.

| round | load | encode | geometry | staging | **recording** | recording's share |
|---|---:|---:|---:|---:|---:|---:|
| 1 A — span | 7.2 | 52.715 ms | 43.444 | 5.716 | 3.479 | **6.6 %** |
| 1 B — none | 5.5 | 50.406 | 41.316 | 5.561 | 3.474 | **6.9 %** |
| 2 A — span | 4.1 | 53.211 | 43.460 | 6.186 | 3.564 | **6.7 %** |
| 2 B — none | 4.0 | 50.538 | 41.343 | 5.506 | 3.559 | **7.3 %** |
| 3 A — span | 5.8 | 59.024 | 48.100 | 6.499 | 3.812 | **7.4 %** |
| 3 B — none | 14.8 | 56.268 | 46.260 | 5.516 | 4.226 | **7.7 %** |

**Weak, consistent, and in the direction the instruction count predicts.** Recording's
share is higher without the span in all three pairs — by 0.3, 0.6 and 0.3 points, which at
these encodes is 0.15–0.35 ms against the ~0.3 ms the 4.68 M instructions come to at this
page's overall rate. Nothing here would be evidence on its own; it is a cross-check on a
number that was obtained exactly.

**Two honesty notes about the table.** The absolute encode differs between the two builds
by 2.3–2.8 ms in *both* rounds where the loads matched, which is far more than 1 200 clock
reads can explain (24 µs) and is a code-layout effect of moving a loop into a function —
one more reason to read the shares rather than the durations. And the whole table is one
adapter on one loaded desktop: `doc/HANDOVER.md`'s "24.7 → 10.3 ms" that re-measured as
19.9 → 20.0 is what this paragraph exists to avoid repeating.

---

## 3. Half 2 — the fixture, re-cut

### 3.1 What was wrong

`define_clips` placed clip *j* at `position(j, side × 6)` and `emit` placed mark *i* at
`position(i, side)`, then handed it `clips[i % clips.len()]`. Two grids of different step
and an assignment that ignores both, so a clip's box and its marks' boxes coincided only by
accident: **0 of dense text's 40** and **8 of artwork's 600** clipped commands had a mark
that met the clip clipping it. Until ADR 0057 those marks still got a mark-sized tile,
rasterised, packed, uploaded and multiplied by a residue of zero — so the rows read 40 and
600 tiles and looked like a gate on the residue lane for two ADRs.

### 3.2 What it is now

**A clip is cut around the run of marks that draw under it.** Mark *i* draws under clip
`i × clips / clipped`, so a clip takes three or four consecutive marks on artwork and
twenty on dense text; `marks_box` unions those marks' boxes from the generator's own
arithmetic, and `curve_clip` scales the archetype's ellipse onto that box. The result is
the shape `doc/notes-tiling-bound.md` §3 asked for: **larger than the marks under it,
a fraction of the page, and cutting every one of them** — the ellipse's boundary crosses
each mark it admits, and a stroked mark's expansion reaches half a line width outside the
box, so it is cut at its rim.

Nothing else about the archetypes changed: the same commands, outlines, segments, sizes,
strokes, groups and blends, and the rectangular-clip pages (image page, clip mountain) are
untouched — their clips already covered the page.

### 3.3 What it provably exercises

`a_curve_clip_clips_the_marks_that_draw_under_it`, and the point of it is the general form
of the trap: **when a fixture's subject is an interaction, the gate must fail if the
interaction stops happening.** It asserts from two independent sides —

- **the generator's own arithmetic**, with nothing of the crate in it: all `clipped`
  commands have a mark box meeting their clip's box;
- **the counters**: `tiles == clipped`. A mark whose chain admits no pixel is not
  rasterised at all since ADR 0057, so this is the same statement made through the encode,
  and it is exact on any machine and any adapter.

Plus two weaker invariants that would catch a lane rewired rather than a fixture drifting:
every residue rasterisation is accounted for (`regions + residue tiles ≤ clipped`), and the
page rasterised a residue at all.

**Verified able to fail, both halves separately**, by forcing the defect the gate exists
to catch — `curve_clip` returning the old `position(i, side × 6)`:

```
dense text: the fixture's own arithmetic says only 0 of its 40 clipped commands have a
mark that meets the clip clipping it

dense text: 40 clipped commands and 0 coverage tiles
```

**And the first version of the first half was tautological, which the forcing is what
found.** It compared each mark against `marks_box` — the union of the marks under that
clip — which is the box `curve_clip` is *built from*, so it asserted an identity and passed
happily while the clips were placed on the far side of the page; only the counter half went
red. It now applies `curve_clip`'s own transform to the ellipse and tests the box the scene
will actually carry. A gate on a fixture can be wrong in exactly the way the fixture was,
and running it against the defect is the only thing that says which.

### 3.4 The new rows, and why they are not comparable with the old ones

`tests/archetypes.rs` signature — `(commands, culled, outlines, atlas keys, clip regions,
tiles, layer textures, residue regions, residue tiles, coverage texels)`:

| archetype | before ADR 0057 | after ADR 0057 (2026-08-17 morning) | **re-cut** |
|---|---|---|---|
| dense text | `[4320, 0, 818, 2164, 1, 40, 0, 2, 0, —]` | `[4320, 0, 818, 2164, 1, 0, 0, 0, 0, 0]` | `[4320, 0, 818, 2164, 1, 40, 0, 0, 40, 8 956]` |
| artwork | `[684, 0, 300, 300, 1, 600, 3, 185, 0, —]` | `[684, 0, 300, 300, 1, 8, 3, 2, 6, 12 284]` | `[684, 0, 300, 300, 1, 600, 3, 66, 384, 3 542 360]` |

**None of the three columns is comparable with another, and the reason differs between
them.** From the first to the second, the library changed and the page did not (ADR 0057
stopped rasterising a tile for a mark its chain admits nothing of). From the second to the
third, **the page changed and the library did not**. Only the third column is a measurement
of the residue lane at all: it is the only one where the clips clip.

Three things in the third column are worth reading rather than recording:

- **artwork: 66 regions and 384 per-tile rasterisations.** A run of three or four marks on
  one line keeps a region — it costs less than the tiles it serves — and a run that wraps
  to the next line has a box the width of the grid and is refused one. 66 + 384 = 450
  rasterisations against 600 clipped commands is ADR 0049's mechanism, measured for the
  first time on marks that are actually clipped, **and both branches of its admission rule
  are now gated by one page**.
- **dense text: no region at all, and 40 per-tile rasterisations.** Its clip is twenty
  marks wide and one tall, so the chain's box costs more than the small tiles it would
  serve. That is the rule doing what ADR 0049's own text says it does for "a page whose
  clip is much larger than its marks" — the `q W n` around a line of text.
- **dense text's 8 956 coverage texels are exactly the "40 calls, 8 956 px" ADR 0049's
  Context table measured** with a temporary probe on 2026-08-15. Two independent
  instruments, five days and two ADRs apart, on a page that has been re-cut in between:
  that is the one number in this file that says the re-cut restored the work the old
  fixture appeared to be doing, rather than inventing new work.

---

## 4. Every reader of this page, and whether its conclusion still holds

The archetype page is a shared instrument. Four ADRs and four examples read numbers off it,
and each was checked rather than assumed. **No conclusion is overturned; three
illustrations were weaker than their text implied, and one file was broken.**

| reader | conclusion | what was corrected |
|---|---|---|
| **ADR 0049** — a residue is rasterised once per chain | **holds, and is now demonstrated** | Its 600 → 185 rasterisations and its 37.8 → 28.9 ms were taken on the page whose clips met 8 of 600 marks, so what they measured was largely the removal of repeated rasterisation of tiles that were then multiplied by zero. The mechanism and the saving were real. Two sentences about *which* chains are admitted are now false for the page that exists, and the ADR carries a dated correction saying which. |
| **ADR 0057** — a clipped mark's tile is bounded by its chain's box | **holds; nothing about it changes** | It is what found the defect. Its artwork row (`600 tiles → 8`) is a fact about the old page and is labelled as such where it is quoted. |
| **ADR 0054** — the geometry phase divides, the order does not | **holds, and the re-cut strengthens it** | "artwork is 1.2× because 600 of its 684 marks are residue-clipped and stay serial" is *more* true now: those marks do the serial work the row claimed. Re-run on the re-cut page (`examples/encode_threads.rs`, llvmpipe, load 4–16), artwork's geometry is **60.0 ms at one thread and 62.0 at twenty-four** — it does not divide **at all**, where the old page read 1.2×, while `drawing` still reads 389 → 51 ms (7.7×) in the same run. Its milliseconds are not comparable with anything measured after the re-cut; its conclusion is more sharply true. |
| **ADR 0045** — the hull memo | **holds** | Its artwork reading (−0.21 %, "a wash") was about a page dominated by rasterising 600 tiles; that is still what the page is, so the direction and the conclusion are unchanged and only the absolute instruction counts moved. |
| **ADR 0038** — a plan accumulates in one texture | **holds, untouched** | It reads `layer_textures`, which is **3** before and after: the groups did not change. Its 138.5 → 108.9 ms cold frame is a number about the old page. |
| **ADR 0043** — the warm set learns the surface's format | **holds, untouched** | It reads a *property* (one first-use compile per presenting first frame) on a page that is still layered. |
| `examples/residue_clip.rs` | re-cut | It copies the archetype and was the instrument ADR 0049 measured on, so it was the same defective page. It now prints all three phases and the coverage texels. |
| `examples/encode_threads.rs` | re-cut | Carries the artwork shape for ADR 0054's thread sweep. |
| `examples/surface_measure.rs` | re-cut | Carries the artwork shape for `PLAN.md`'s real-display row, which is now stale and must say so. |
| `examples/retained.rs` | re-cut, **and it was broken** | Its copy of the dense-text row asserted `tiles == 40` and the page had drawn 0 since ADR 0057, so the example panicked at its own gate on `main`. Nothing caught it because `cargo test` does not run examples. It passes again, on a page that draws the 40. |

The rule that follows, and it is the same one twice: **a fixture copied into an example is
a fixture that has to be re-cut in both places, and a signature asserted outside the test
harness is a signature nobody runs.**

---

## 5. What was found and deliberately not done

- **No corpus run.** Nothing this round changes what is drawn: half 1 moves a clock span
  (which nothing outside `Timings::phases` can observe) and half 2 changes a test fixture
  and four examples. `Device::render` is byte-for-byte the same function it was.
- **`examples/retained.rs`'s stale signature is fixed but not gated.** The general problem
  — four examples carrying private copies of a fixture that `tests/archetypes.rs` owns — is
  a round of its own: the shape would be a shared, non-`dev` fixture module the examples
  and the test both read, and it is not free, because an example that imports a test's
  module is not something cargo will do without moving the generator into the crate.
- **`Options::instrument_encode` still has one detail level.**
  `doc/notes-recording-shares.md` §5's recommendation — `encode: bounds`, `encode: atlas`,
  `encode: instances`, `encode: commit` behind a second flag, with `encode: recording` kept
  as the remainder — is what would trip ADR 0023's "revisit when", and it is still owed to
  the caller. This round only put an existing seam in the right place.
- **`push_coverage_styled`'s triangle-count sum and `polyline_bounds` were left in
  `recording`**, deliberately and with the reason written into ADR 0023's amendment. They
  are per-point work, and per-point is not the criterion.
- **The wall-clock A/B was not chased past three rounds.** It agrees in direction and in
  magnitude with the instruction count and it cannot do better than that on a machine whose
  load average moved by two orders of magnitude inside the hour this round ran. The
  instruction count answers the same question exactly, which is `doc/HANDOVER.md`'s split
  between "how many" and "how fast" deciding where to stop rather than an abandoned
  measurement.
- **`no_archetype_takes_absurdly_long`'s threshold was re-taken with the page** and is
  still the loosest gate in the file — a multiple, not a bound, and `#[ignore]`d, for the
  reason its own doc comment gives. Artwork is now the worst archetype by a wide margin
  because it rasterises 3.5 million coverage texels where it rasterised 12 284.
